import { createResource, For, createEffect, Accessor } from 'solid-js';

interface Comment {
  id: number;
  text: string;
}

interface RecentCommentsProps {
  refreshTrigger?: Accessor<any>;
}

const fetchComments = async () => {
  try {
    const response = await fetch('/api/comments');
    if (response.ok) {
      return (await response.json()) as Comment[];
    }
  } catch (err) {
    console.error('Error fetching comments:', err);
  }
  return [];
};

export default function RecentComments(props: RecentCommentsProps) {
  const [recentComments, { refetch }] = createResource(
    () => props.refreshTrigger?.(),
    fetchComments,
    { initialValue: [] }
  );

  return (
    <div>
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <h3>Recent Comments</h3>
        <button onClick={() => refetch()} disabled={recentComments.loading}>
          {recentComments.loading ? 'Refreshing...' : 'Refresh'}
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

