use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage};
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_mqtt_broker::MqttBrokerConfig;

use super::*;

/// Test-only blob provider: in-memory backend, effectively unlimited
/// quota.
pub(crate) fn test_blob_provider() -> Arc<dyn BlobProvider> {
    Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None))
}

/// Test-only messaging context: a real (but throwaway, no network
/// listener) broker with no engine backreference -- sufficient for
/// tests that don't exercise guest-delivery messaging.
pub(crate) fn test_messaging_context() -> MessagingContext {
    MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    }
}

/// Test-only streaming context: a mock in-memory `EndpointRegistry` with
/// no engine backreference -- sufficient for tests that don't exercise
/// stream-protocol registration/routing.
pub(crate) fn test_streaming_context() -> StreamContext {
    StreamContext {
        registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        engine: Weak::new(),
    }
}

/// Test-only proxy handle: always-unavailable -- sufficient for tests
/// that don't exercise `syneroym:proxy/proxy::call`.
pub(crate) fn test_service_proxy() -> Weak<dyn ServiceProxy> {
    super::empty_service_proxy()
}

mod tests_proxy;
mod tests_services;
mod tests_store;
