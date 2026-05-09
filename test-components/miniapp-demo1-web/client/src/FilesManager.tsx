import { createSignal, createResource, For } from 'solid-js';

type FileInfo = {
  name: string;
  size: number;
};

const fetchFiles = async () => {
  const res = await fetch('/api/files');
  return (await res.json()) as FileInfo[];
};

function FilesManager() {
  const [files, { refetch }] = createResource(fetchFiles);
  const [uploadStatus, setUploadStatus] = createSignal('');

  const handleUpload = async (e: Event) => {
    e.preventDefault();
    const input = document.getElementById('fileInput') as HTMLInputElement;
    if (!input.files || input.files.length === 0) return;

    const formData = new FormData();
    formData.append('file', input.files[0]);

    setUploadStatus('Uploading...');
    try {
      const res = await fetch('/api/files', {
        method: 'POST',
        body: formData,
      });
      if (res.ok) {
        setUploadStatus('Upload successful!');
        input.value = ''; // Reset input
        refetch();
      } else {
        setUploadStatus('Upload failed.');
      }
    } catch (err) {
      console.error(err);
      setUploadStatus('Network error.');
    }
  };

  return (
    <div>
      <h3>Files</h3>
      <form onSubmit={handleUpload}>
        <input type="file" id="fileInput" style="margin-bottom: 10px;" />
        <br />
        <button type="submit">Upload</button>
      </form>
      {uploadStatus() && <p>{uploadStatus()}</p>}

      <ul style="margin-top: 20px;">
        <For each={files()}>{(file) =>
          <li>
            <a href={`/api/files/${file.name}`} target="_blank">{file.name}</a>
            <span style="margin-left: 10px; color: #666;">({file.size} bytes)</span>
          </li>
        }</For>
      </ul>
    </div>
  );
}

export default FilesManager;
