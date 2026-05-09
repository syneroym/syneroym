import { createSignal, onMount, onCleanup } from 'solid-js';

export default function LastUpdated() {
  const [lastUpdated, setLastUpdated] = createSignal<string>('Never');
  const [recdMsg, setRecdMsg] = createSignal<string>('');
  const [status, setStatus] = createSignal('Connecting...');

  let socket: WebSocket | undefined;

  onMount(() => {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/ws`;

    socket = new WebSocket(wsUrl);

    socket.onopen = () => {
      setStatus('Connected');

      const intervalId = setInterval(() => {
        if (socket?.readyState === WebSocket.OPEN) {
          socket.send("Hi from client");
        }
      }, 10000);

      onCleanup(() => clearInterval(intervalId));
    };

    socket.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.commentUpdateTimestamp) {
          setLastUpdated(new Date(data.commentUpdateTimestamp).toLocaleString());
        }
        if (data.recdMsg) {
          setRecdMsg(data.recdMsg);
        }
      } catch (e) {
        console.error('Failed to parse WS message', e);
      }
    };

    socket.onclose = () => {
      setStatus('Disconnected');
    };

    socket.onerror = (error) => {
      console.error('WebSocket error:', error);
      setStatus('Error');
    };
  });

  onCleanup(() => {
    if (socket) {
      socket.close();
    }
  });

  return (
    <div style="background: #e6f7ff; padding: 10px; margin-bottom: 20px; border: 1px solid #91d5ff; border-radius: 4px;">
      <strong>Live Updates:</strong> {status()}
      <br />
      Last comment added at: <span>{lastUpdated()}</span>
      <br />
      Received Msg: <span>{recdMsg()}</span>
    </div>
  );
}