use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use dashmap::DashMap;
use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SetRecorderError, SharedString, Unit,
};
use serde::{Deserialize, Serialize};

static GLOBAL_RECORDER: OnceLock<MemoryRecorder> = OnceLock::new();

/// An in-memory metrics recorder that implements the `metrics::Recorder` trait.
///
/// **Architecture Note**:
/// This recorder captures counters, gauges, and histograms internally using
/// thread-safe DashMaps. Instead of pushing metrics directly to an external
/// service (like Prometheus or Datadog), it enables an active-pull or
/// snapshotting model. The Substrate control plane can invoke `.snapshot()` to
/// retrieve the current state and serve it via HTTP (e.g., `/metrics` endpoint)
/// or periodically push it to an aggregator.
#[derive(Debug, Default, Clone)]
pub struct MemoryRecorder {
    counters: Arc<DashMap<String, Arc<Mutex<u64>>>>,
    gauges: Arc<DashMap<String, Arc<Mutex<f64>>>>,
    histograms: Arc<DashMap<String, Arc<Mutex<Vec<f64>>>>>,
}

struct InnerCounter(Arc<Mutex<u64>>);
impl CounterFn for InnerCounter {
    fn increment(&self, value: u64) {
        if let Ok(mut g) = self.0.lock() {
            *g += value;
        }
    }
    fn absolute(&self, value: u64) {
        if let Ok(mut g) = self.0.lock() {
            *g = value;
        }
    }
}

struct InnerGauge(Arc<Mutex<f64>>);
impl GaugeFn for InnerGauge {
    fn increment(&self, value: f64) {
        if let Ok(mut g) = self.0.lock() {
            *g += value;
        }
    }
    fn decrement(&self, value: f64) {
        if let Ok(mut g) = self.0.lock() {
            *g -= value;
        }
    }
    fn set(&self, value: f64) {
        if let Ok(mut g) = self.0.lock() {
            *g = value;
        }
    }
}

struct InnerHistogram(Arc<Mutex<Vec<f64>>>);
impl HistogramFn for InnerHistogram {
    fn record(&self, value: f64) {
        if let Ok(mut g) = self.0.lock() {
            g.push(value);
        }
    }
}

impl MemoryRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install this recorder as the global recorder.
    pub fn install(self) -> Result<(), SetRecorderError<Self>> {
        let recorder_to_set = self.clone();
        match metrics::set_global_recorder(self) {
            Ok(()) => {
                let _ = GLOBAL_RECORDER.set(recorder_to_set);
                Ok(())
            }
            Err(e) => {
                if GLOBAL_RECORDER.get().is_some() {
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Retrieve the globally installed MemoryRecorder.
    pub fn global() -> Option<MemoryRecorder> {
        GLOBAL_RECORDER.get().cloned()
    }

    /// Snapshot the current metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut counters = HashMap::new();
        for r in self.counters.iter() {
            let lock: &Mutex<u64> = r.value();
            if let Ok(g) = lock.lock() {
                counters.insert(r.key().clone(), *g);
            }
        }

        let mut gauges = HashMap::new();
        for r in self.gauges.iter() {
            let lock: &Mutex<f64> = r.value();
            if let Ok(g) = lock.lock() {
                gauges.insert(r.key().clone(), *g);
            }
        }

        let mut histograms = HashMap::new();
        for r in self.histograms.iter() {
            let lock: &Mutex<Vec<f64>> = r.value();
            if let Ok(g) = lock.lock() {
                let mut values: Vec<f64> = g.clone();
                if values.is_empty() {
                    continue;
                }
                values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

                let count = values.len();
                let sum: f64 = values.iter().sum();
                let min = values[0];
                let max = values[count - 1];

                let p50 = values[(count as f64 * 0.5) as usize];
                let p95 = values[((count as f64 * 0.95) as usize).min(count - 1)];
                let p99 = values[((count as f64 * 0.99) as usize).min(count - 1)];

                histograms.insert(
                    r.key().clone(),
                    HistogramSnapshot { count, sum, min, max, p50, p95, p99 },
                );
            }
        }

        MetricsSnapshot { counters, gauges, histograms }
    }
}

impl Recorder for MemoryRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        let name = key.name().to_string();
        let entry = self.counters.entry(name).or_insert_with(|| Arc::new(Mutex::new(0)));
        Counter::from_arc(Arc::new(InnerCounter(entry.clone())))
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        let name = key.name().to_string();
        let entry = self.gauges.entry(name).or_insert_with(|| Arc::new(Mutex::new(0.0)));
        Gauge::from_arc(Arc::new(InnerGauge(entry.clone())))
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        let name = key.name().to_string();
        let entry = self.histograms.entry(name).or_insert_with(|| Arc::new(Mutex::new(Vec::new())));
        Histogram::from_arc(Arc::new(InnerHistogram(entry.clone())))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    pub count: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub counters: HashMap<String, u64>,
    pub gauges: HashMap<String, f64>,
    pub histograms: HashMap<String, HistogramSnapshot>,
}

#[cfg(test)]
mod tests {
    use metrics::Level;

    use super::*;

    #[test]
    fn test_memory_recorder_snapshots() {
        let recorder = MemoryRecorder::new();
        let metadata = Metadata::new("test", Level::INFO, Some("test"));

        let c_key = Key::from_name("test_counter");
        let c = recorder.register_counter(&c_key, &metadata);
        c.increment(5);
        c.increment(10);

        let g_key = Key::from_name("test_gauge");
        let g = recorder.register_gauge(&g_key, &metadata);
        g.set(42.5);
        g.increment(1.5);

        let h_key = Key::from_name("test_histogram");
        let h = recorder.register_histogram(&h_key, &metadata);
        for &val in &[10.0, 20.0, 30.0, 40.0, 50.0] {
            h.record(val);
        }

        let snap = recorder.snapshot();

        assert_eq!(*snap.counters.get("test_counter").unwrap(), 15);
        assert_eq!(*snap.gauges.get("test_gauge").unwrap(), 44.0);

        let h_snap = snap.histograms.get("test_histogram").unwrap();
        assert_eq!(h_snap.count, 5);
        assert_eq!(h_snap.sum, 150.0);
        assert_eq!(h_snap.min, 10.0);
        assert_eq!(h_snap.max, 50.0);
        assert_eq!(h_snap.p50, 30.0);
    }
}
