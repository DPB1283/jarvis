// PC health watch: every minute, check RAM, disks, GPU temperature (NVIDIA) and runaway apps.
// Warns once per issue per 30 min. In game mode, warnings are held and summarised afterwards.
import { z } from "zod";
import { bus } from "./bus";
import { getSettings } from "./config";
import { isGameMode } from "./gamemode";
import { runPs } from "./tools/ps";
import { defineTool } from "./tools/types";

interface Snapshot {
  cpuLoad: number;
  ramUsedPct: number;
  ramTotalGB: number;
  disks: { drive: string; freeGB: number; sizeGB: number }[];
  topApps: { name: string; memGB: number; cpuSec: number }[];
  gpu: { tempC: number; utilPct: number; memUsedGB: number; memTotalGB: number } | null;
}

async function snapshot(): Promise<Snapshot> {
  const r = await runPs(
    `$os = Get-CimInstance Win32_OperatingSystem
    $cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
    $disks = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | ForEach-Object { @{ drive = $_.DeviceID; freeGB = [math]::Round($_.FreeSpace/1GB,1); sizeGB = [math]::Round($_.Size/1GB,1) } }
    $top = Get-Process | Group-Object ProcessName | ForEach-Object { @{ name = $_.Name; memGB = [math]::Round((($_.Group | Measure-Object WorkingSet64 -Sum).Sum)/1GB,2); cpuSec = [math]::Round((($_.Group | Measure-Object CPU -Sum).Sum),0) } } | Sort-Object { $_.memGB } -Descending | Select-Object -First 5
    $gpu = $null
    $smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
    if ($smi) { $g = (& nvidia-smi --query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total --format=csv,noheader,nounits | Select-Object -First 1) -split ',\\s*'
      if ($g.Count -ge 4) { $gpu = @{ tempC = [int]$g[0]; utilPct = [int]$g[1]; memUsedGB = [math]::Round([double]$g[2]/1024,1); memTotalGB = [math]::Round([double]$g[3]/1024,1) } } }
    @{ cpuLoad = [int]$cpu; ramUsedPct = [math]::Round(100 - 100*$os.FreePhysicalMemory/$os.TotalVisibleMemorySize, 0); ramTotalGB = [math]::Round($os.TotalVisibleMemorySize/1MB,1); disks = @($disks); topApps = @($top); gpu = $gpu } | ConvertTo-Json -Depth 4 -Compress`,
    30_000,
  );
  if (r.code !== 0) throw new Error(r.stderr || "health check failed");
  return JSON.parse(r.stdout) as Snapshot;
}

function describe(s: Snapshot): string {
  return [
    `CPU load ${s.cpuLoad}%`,
    `RAM ${s.ramUsedPct}% used of ${s.ramTotalGB} GB`,
    s.gpu ? `GPU ${s.gpu.tempC}°C, ${s.gpu.utilPct}% busy, VRAM ${s.gpu.memUsedGB}/${s.gpu.memTotalGB} GB` : "GPU temperature unavailable (no NVIDIA tools)",
    `Disks: ${s.disks.map((d) => `${d.drive} ${d.freeGB} GB free of ${d.sizeGB} GB`).join("; ")}`,
    `Biggest apps by memory: ${s.topApps.map((a) => `${a.name} ${a.memGB} GB`).join(", ")}`,
  ].join("\n");
}

export const pcHealth = defineTool({
  name: "pc_health",
  description: "Check the PC's health right now: CPU load, RAM, GPU temperature/usage (NVIDIA), disk space, and which apps use the most memory. CPU temperature isn't readable without extra tools.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check the PC's health",
  run: async () => describe(await snapshot()),
});

export function startHealthWatch(): () => void {
  const lastWarned = new Map<string, number>();
  const strikes = new Map<string, number>();
  const held: string[] = [];
  const COOLDOWN = 30 * 60_000;

  const warn = (key: string, text: string, needStrikes = 1) => {
    const n = (strikes.get(key) ?? 0) + 1;
    strikes.set(key, n);
    if (n < needStrikes || Date.now() - (lastWarned.get(key) ?? 0) < COOLDOWN) return;
    lastWarned.set(key, Date.now());
    if (isGameMode()) {
      if (!held.includes(text)) held.push(text);
      return;
    }
    bus.emit("toast", { level: "warn", text });
    bus.emit("speak", { text: `Pardon me, ${getSettings().userTitle}. ${text}` });
    bus.emit("speak_end", {});
  };
  const clear = (key: string) => strikes.delete(key);

  const tick = async () => {
    if (!getSettings().health?.warnings) return;
    let s: Snapshot;
    try {
      s = await snapshot();
    } catch {
      return;
    }
    if (s.ramUsedPct >= 92) {
      const top = s.topApps[0];
      warn("ram", `Memory is ${s.ramUsedPct}% full${top ? `; ${top.name} is using ${top.memGB} gigabytes` : ""}. Shall I close something?`, 2);
    } else clear("ram");
    for (const d of s.disks) {
      if (d.freeGB < 10 || d.freeGB / d.sizeGB < 0.05) warn(`disk-${d.drive}`, `Drive ${d.drive.replace(":", "")} is nearly full, only ${d.freeGB} gigabytes left.`);
      else clear(`disk-${d.drive}`);
    }
    if (s.gpu && s.gpu.tempC >= 85) warn("gpu-heat", `Your graphics card is running hot, ${s.gpu.tempC} degrees.`, 2);
    else clear("gpu-heat");
    const hog = s.topApps.find((a) => a.memGB >= 6 && !/^(memory compression|system|vmmem)$/i.test(a.name));
    if (hog) warn(`hog-${hog.name}`, `${hog.name} is using ${hog.memGB} gigabytes of memory.`, 3);
  };

  const offGame = bus.on("game_mode", ({ active }) => {
    if (active || !held.length) return;
    const text = `While you were playing: ${held.splice(0).join(" ")}`;
    bus.emit("toast", { level: "warn", text });
    bus.emit("speak", { text });
    bus.emit("speak_end", {});
  });
  const timer = setInterval(() => void tick(), 60_000);
  setTimeout(() => void tick(), 20_000);
  return () => {
    clearInterval(timer);
    offGame();
  };
}
