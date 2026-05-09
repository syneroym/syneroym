import { createSignal } from 'solid-js';
import RecentComments from './RecentComments';
import LastUpdated from './LastUpdated';
import FilesManager from './FilesManager';

function App() {
  const [comment, setComment] = createSignal('');
  const [status, setStatus] = createSignal('');
  // Trigger to refresh the recent comments list
  const [refreshTrigger, setRefreshTrigger] = createSignal(0);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setStatus('Submitting...');
    try {
      const response = await fetch('/api/comments', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ text: comment() }),
      });
      if (response.ok) {
        setStatus('Comment saved!');
        setComment('');
        // Trigger refresh of the comments list
        setRefreshTrigger(t => t + 1);
      } else {
        setStatus('Error saving comment.');
      }
    } catch (err) {
      console.error(err);
      setStatus('Network error.');
    }
  };

  return (
    <div style="padding: 20px; font-family: sans-serif; max-width: 600px; margin: 0 auto;">
      <h2>Comments</h2>
      <form onSubmit={handleSubmit}>
        <textarea
          value={comment()}
          onInput={(e) => setComment(e.currentTarget.value)}
          placeholder="Write a comment..."
          rows={4}
          style="width: 100%; display: block; margin-bottom: 10px;"
        />
        <button type="submit">Submit</button>
      </form>
      {status() && <p>{status()}</p>}
      
      <hr style="margin: 20px 0;" />
      
      <LastUpdated />
      <RecentComments refreshTrigger={refreshTrigger} />
      
      <hr style="margin: 20px 0;" />
      
      <FilesManager />
      
      <br />
      <a href="/">Back to Home</a>
    </div>
  );
}
export default App;