Add-Type -AssemblyName System.Windows.Forms, System.Drawing
      if ([System.Windows.Forms.Clipboard]::ContainsText()) { "TEXT"; [System.Windows.Forms.Clipboard]::GetText(); exit }
      if ([System.Windows.Forms.Clipboard]::ContainsFileDropList()) { "FILES"; [System.Windows.Forms.Clipboard]::GetFileDropList(); exit }
      if ([System.Windows.Forms.Clipboard]::ContainsImage()) {
        $img = [System.Windows.Forms.Clipboard]::GetImage()
        $scale = [Math]::Min([double]1.0, [Math]::Min([double]1568 / [double][Math]::Max($img.Width, $img.Height), [Math]::Sqrt([double]1150000 / ([double]$img.Width * [double]$img.Height))))
        $w = [int]($img.Width * $scale); $h = [int]($img.Height * $scale)
        $bmp = New-Object System.Drawing.Bitmap $img, $w, $h
        $ms = New-Object System.IO.MemoryStream; $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Jpeg)
        "IMAGE"; [Convert]::ToBase64String($ms.ToArray()); exit }
      "EMPTY"
