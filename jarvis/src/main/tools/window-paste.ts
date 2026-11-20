import { ps, psq, runPs, WIN_NATIVE } from "./ps";
const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

export async function foregroundWindow(): Promise<{ pid: number; title: string }> {
  const out = await ps(`${WIN_NATIVE}
    $h = [JarvisNative]::GetForegroundWindow(); $p = 0; [void][JarvisNative]::GetWindowThreadProcessId($h, [ref]$p)
    "$p|$([JarvisNative]::ForegroundTitle())"`);
  const [pid, ...title] = out.split("|");
  return { pid: Number(pid), title: title.join("|") };
}

/** CF_HTML header offsets count UTF-8 bytes, including the header itself. */
function cfHtml(fragment: string): Buffer {
  const pre = "<html><head><meta charset=\"utf-8\"></head><body><!--StartFragment-->";
  const post = "<!--EndFragment--></body></html>";
  const header = (a: number, b: number, c: number, d: number) =>
    `Version:0.9\r\nStartHTML:${String(a).padStart(10, "0")}\r\nEndHTML:${String(b).padStart(10, "0")}\r\nStartFragment:${String(c).padStart(10, "0")}\r\nEndFragment:${String(d).padStart(10, "0")}\r\n`;
  const headerLen = Buffer.byteLength(header(0, 0, 0, 0));
  const startHtml = headerLen;
  const startFrag = startHtml + Buffer.byteLength(pre);
  const endFrag = startFrag + Buffer.byteLength(fragment);
  const endHtml = endFrag + Buffer.byteLength(post);
  return Buffer.from(header(startHtml, endHtml, startFrag, endFrag) + pre + fragment + post, "utf8");
}

export async function pasteRich(html: string, text: string, expectPid?: number): Promise<void> {
  if (expectPid !== undefined) await ensureForeground(expectPid);
  const r = await runPs(
    `Add-Type -AssemblyName System.Windows.Forms
    $saved = New-Object System.Windows.Forms.DataObject
    $hadData = $false
    $old = [System.Windows.Forms.Clipboard]::GetDataObject()
    if ($old) { foreach ($f in $old.GetFormats($false)) { try { $d = $old.GetData($f); if ($null -ne $d) { $saved.SetData($f, $d); $hadData = $true } } catch {} } }
    $do = New-Object System.Windows.Forms.DataObject
    $bytes = [Convert]::FromBase64String('${cfHtml(html).toString("base64")}')
    $do.SetData("HTML Format", (New-Object System.IO.MemoryStream (,$bytes)))
    $do.SetData([System.Windows.Forms.DataFormats]::UnicodeText, ${psq(text)})
    [System.Windows.Forms.Clipboard]::SetDataObject($do, $true)
    Start-Sleep -Milliseconds 60
    [System.Windows.Forms.SendKeys]::SendWait("^v")
    Start-Sleep -Milliseconds 1200
    if ($hadData) { [System.Windows.Forms.Clipboard]::SetDataObject($saved, $true) } else { [System.Windows.Forms.Clipboard]::Clear() }`,
    60_000,
  );
  if (r.code !== 0) throw new Error(`Couldn't paste: ${r.stderr || "clipboard busy"}`);
}

export async function ensureForeground(pid: number): Promise<void> {
  if ((await foregroundPid()) === pid) return;
  await focusWindow(pid);
  await sleep(300);
  if ((await foregroundPid()) !== pid) {
    throw new Error("The target app isn't the active window (Windows blocked switching to it), so I stopped without typing anything.");
  }
}

export async function foregroundPid(): Promise<number> {
  const out = await ps(`${WIN_NATIVE}
    $h = [JarvisNative]::GetForegroundWindow(); $p = 0; [void][JarvisNative]::GetWindowThreadProcessId($h, [ref]$p); $p`);
  return Number(out);
}

export async function focusWindow(pid: number): Promise<void> {
  await ps(`${WIN_NATIVE}
    $p = Get-Process -Id ${pid}
    [void][JarvisNative]::Focus($p.MainWindowHandle)
  `);
}

