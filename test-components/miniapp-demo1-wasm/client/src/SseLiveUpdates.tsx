import { createSignal, onMount, onCleanup } from 'solid-js';

export default function SseLiveUpdates() {
  const [lastUpdated, setLastUpdated] = createSignal<string>('Never');
  const [status, setStatus] = createSignal('Connecting...');

  let eventSource: EventSource | undefined;

  onMount(() => {
    eventSource = new EventSource('/api/events');

    eventSource.onopen = () => {
      setStatus('Connected');
    };

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.commentUpdateTimestamp) {
          setLastUpdated(new Date(data.commentUpdateTimestamp).toLocaleString());
        }
      } catch (e) {
        console.error('Failed to parse SSE message', e);
      }
    };

    eventSource.onerror = (error) => {
      console.error('SSE error:', error);
      setStatus('Error');
    };
  });

  onCleanup(() => {
    if (eventSource) {
      eventSource.close();
    }
  });

  return (
    <div style="background: #f6ffed; padding: 10px; margin-bottom: 20px; border: 1px solid #b7eb8f; border-radius: 4px;">
      <strong>SSE Updates:</strong> {status()}
      <br />
      Last comment added at (SSE): <span>{lastUpdated()}</span>
    </div>
  );
}
