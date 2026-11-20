$os = Get-CimInstance Win32_OperatingSystem
    $cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
    $disks = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | ForEach-Object { @{ drive = $_.DeviceID; freeGB = [math]::Round($_.FreeSpace/1GB,1); sizeGB = [math]::Round($_.Size/1GB,1) } }
    $top = Get-Process | Group-Object ProcessName | ForEach-Object { @{ name = $_.Name; memGB = [math]::Round((($_.Group | Measure-Object WorkingSet64 -Sum).Sum)/1GB,2); cpuSec = [math]::Round((($_.Group | Measure-Object CPU -Sum).Sum),0) } } | Sort-Object { $_.memGB } -Descending | Select-Object -First 5
    $gpu = $null
    $smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
    if ($smi) { $g = (& nvidia-smi --query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total --format=csv,noheader,nounits | Select-Object -First 1) -split ',\s*'
      if ($g.Count -ge 4) { $gpu = @{ tempC = [int]$g[0]; utilPct = [int]$g[1]; memUsedGB = [math]::Round([double]$g[2]/1024,1); memTotalGB = [math]::Round([double]$g[3]/1024,1) } } }
    @{ cpuLoad = [int]$cpu; ramUsedPct = [math]::Round(100 - 100*$os.FreePhysicalMemory/$os.TotalVisibleMemorySize, 0); ramTotalGB = [math]::Round($os.TotalVisibleMemorySize/1MB,1); disks = @($disks); topApps = @($top); gpu = $gpu } | ConvertTo-Json -Depth 4 -Compress
