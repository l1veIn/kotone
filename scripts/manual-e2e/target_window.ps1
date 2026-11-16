param([string]$Dir)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force $Dir | Out-Null
$log = [System.IO.StreamWriter]::new((Join-Path $Dir "events.log"), $false, [System.Text.UTF8Encoding]::new($false))
$log.AutoFlush = $true
function Now { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
function Write-Event([string]$text) { $log.WriteLine("$(Now) $text") }

$form = New-Object System.Windows.Forms.Form
$form.Text = "KotoneScriptTarget"
$form.Name = "form"
$form.StartPosition = "Manual"
$form.Location = New-Object System.Drawing.Point(120, 120)
$form.ClientSize = New-Object System.Drawing.Size(800, 480)
$form.BackColor = [System.Drawing.Color]::White
$form.KeyPreview = $true
$form.ImeMode = [System.Windows.Forms.ImeMode]::Disable

$panel = New-Object System.Windows.Forms.Panel
$panel.Location = New-Object System.Drawing.Point(0, 0)
$panel.Size = New-Object System.Drawing.Size(160, 72)
$panel.BackColor = [System.Drawing.Color]::FromArgb(210, 210, 210)
$form.Controls.Add($panel)

$label = New-Object System.Windows.Forms.Label
$label.Text = "找到对局"
$label.Font = New-Object System.Drawing.Font("Microsoft YaHei", 40)
$label.AutoSize = $true
$label.Location = New-Object System.Drawing.Point(260, 260)
$label.ForeColor = [System.Drawing.Color]::Black
$form.Controls.Add($label)


$icon = New-Object System.Windows.Forms.Panel
$icon.Location = New-Object System.Drawing.Point(600, 60)
$icon.Size = New-Object System.Drawing.Size(60, 60)
$icon.BackColor = [System.Drawing.Color]::FromArgb(255, 140, 0)
$dot = New-Object System.Windows.Forms.Panel
$dot.Location = New-Object System.Drawing.Point(20, 20)
$dot.Size = New-Object System.Drawing.Size(20, 20)
$dot.BackColor = [System.Drawing.Color]::FromArgb(0, 70, 220)
$icon.Controls.Add($dot)
$form.Controls.Add($icon)

$accept = New-Object System.Windows.Forms.Label
$accept.Text = "接受"
$accept.Font = New-Object System.Drawing.Font("Microsoft YaHei", 28)
$accept.AutoSize = $true
$accept.Location = New-Object System.Drawing.Point(100, 380)
$accept.ForeColor = [System.Drawing.Color]::Black
$form.Controls.Add($accept)

function Ratio() {
  $p = $form.PointToClient([System.Windows.Forms.Cursor]::Position)
  "{0:0.000} {1:0.000}" -f ($p.X / $form.ClientSize.Width), ($p.Y / $form.ClientSize.Height)
}
$down = { param($s, $e) Write-Event "MOUSE $($e.Button) $(Ratio) on=$($s.Name)$($s.Text)" }
$up = { param($s, $e) Write-Event "MOUSEUP $($e.Button) $(Ratio)" }
foreach ($c in @($form, $panel, $label, $icon, $dot, $accept)) { $c.Add_MouseDown($down); $c.Add_MouseUp($up) }
$form.Add_MouseWheel({ param($s, $e) Write-Event "WHEEL $($e.Delta)" })

$form.Add_KeyDown({ param($s, $e) Write-Event "DOWN $($e.KeyCode)"; $e.Handled = $true; $e.SuppressKeyPress = $true })
$form.Add_KeyUp({ param($s, $e) Write-Event "UP $($e.KeyCode)" })
$form.Add_Activated({ Write-Event "FOCUS gained" })
$form.Add_Deactivate({ Write-Event "FOCUS lost" })

$other = New-Object System.Windows.Forms.Form
$other.Text = "KotoneScriptOther"
$other.StartPosition = "Manual"
$other.Location = New-Object System.Drawing.Point(980, 120)
$other.ClientSize = New-Object System.Drawing.Size(300, 200)
$other.KeyPreview = $true
$other.ImeMode = [System.Windows.Forms.ImeMode]::Disable
$other.Add_KeyDown({ param($s, $e) Write-Event "OTHER-DOWN $($e.KeyCode)"; $e.SuppressKeyPress = $true })

$control = Join-Path $Dir "control.txt"
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 50
$script:refocusAt = 0
$timer.Add_Tick({
    if ($script:refocusAt -gt 0 -and (Now) -ge $script:refocusAt) {
        $script:refocusAt = 0
        $form.Activate()
        Write-Event "CMD refocus"
    }
    if (Test-Path $control) {
        $cmd = (Get-Content -Raw $control).Trim()
        Remove-Item $control -Force
        $parts = $cmd -split " "
        switch ($parts[0]) {
            "dark" { $panel.BackColor = [System.Drawing.Color]::Black; Write-Event "CMD dark" }
            "light" { $panel.BackColor = [System.Drawing.Color]::FromArgb(210, 210, 210); Write-Event "CMD light" }
            "blur" { if (-not $other.Visible) { $other.Show() }; $other.Activate(); $script:refocusAt = (Now) + [int]$parts[1]; Write-Event "CMD blur" }
            "fg" { $form.Activate(); Write-Event "CMD fg" }
            "close" { Write-Event "CMD close"; $form.Close() }
        }
    }
})
$timer.Start()
$form.Add_Shown({ $form.Activate(); Set-Content -Path (Join-Path $Dir "ready.txt") -Value $form.Handle; Write-Event "READY" })
[System.Windows.Forms.Application]::Run($form)
$log.Close()
