import { createSignal, onMount, For, createEffect, Accessor } from 'solid-js';

interface Comment {
  id: number;
  text: string;
}

interface RecentCommentsProps {
  refreshTrigger?: Accessor<any>;
}

export default function RecentComments(props: RecentCommentsProps) {
  const [recentComments, setRecentComments] = createSignal<Comment[]>([]);
  const [loading, setLoading] = createSignal(false);

  const fetchComments = async () => {
    setLoading(true);
    try {
      const response = await fetch('/api/comments');
      if (response.ok) {
        const data = await response.json();
        setRecentComments(data);
      } else {
        console.error('Failed to fetch comments');
      }
    } catch (err) {
      console.error('Error fetching comments:', err);
    } finally {
        setLoading(false);
    }
  };

  onMount(() => {
    fetchComments();
  });
  
  createEffect(() => {
    if (props.refreshTrigger) {
        props.refreshTrigger(); // track dependency
        fetchComments();
    }
  });

  return (
    <div>
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <h3>Recent Comments</h3>
        <button onClick={fetchComments} disabled={loading()}>
            {loading() ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>
      
      <ul style="list-style: none; padding: 0;">
        <For each={recentComments()} fallback={<p>No comments yet.</p>}>
          {(item) => (
            <li style="background: #f0f0f0; margin-bottom: 10px; padding: 10px; border-radius: 4px;">
              {item.text}
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}
