document.querySelector('#pair').addEventListener('click', async () => {
  const port = Number(document.querySelector('#port').value);
  const code = document.querySelector('#code').value.trim();
  const status = document.querySelector('#status');
  if (!Number.isInteger(port) || port < 1 || port > 65535 || !/^[a-f0-9]{64}$/i.test(code)) {
    status.textContent = 'Enter the port and pairing code from Jarvis Settings.';
    return;
  }
  const result = await chrome.runtime.sendMessage({ type: 'pair', port, code });
  document.querySelector('#code').value = '';
  status.textContent = result?.ok ? 'Pairing requested. Check the connection status in Jarvis Settings.' : 'Could not start pairing.';
});
