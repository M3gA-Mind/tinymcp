$ErrorActionPreference = 'Stop'

$root = (Get-Location).Path
$source = Join-Path $root 'target/debug/tinymcp.dll'
$destination = Join-Path $root 'target/module-e2e-private'

if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw "expected built TinyMCP module at $source"
}

if (Test-Path -LiteralPath $destination) {
    Remove-Item -LiteralPath $destination -Recurse -Force
}
New-Item -ItemType Directory -Path $destination | Out-Null

# Match the release workflow's trusted artifact directory: the current runner
# identity owns it, and only that identity, LocalSystem, and Administrators
# receive full control. Do not weaken TinyBus's ACL admission policy.
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$security = [System.Security.AccessControl.DirectorySecurity]::new()
$security.SetOwner($identity.User)
$security.SetAccessRuleProtection($true, $false)
$rights = [System.Security.AccessControl.FileSystemRights]::FullControl
$inheritance = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
$propagation = [System.Security.AccessControl.PropagationFlags]::None
$access = [System.Security.AccessControl.AccessControlType]::Allow
foreach ($sidValue in @(
    $identity.User.Value,
    'S-1-5-18',
    'S-1-5-32-544'
)) {
    $sid = [System.Security.Principal.SecurityIdentifier]::new($sidValue)
    $rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $sid,
        $rights,
        $inheritance,
        $propagation,
        $access
    )
    [void]$security.AddAccessRule($rule)
}
Set-Acl -LiteralPath $destination -AclObject $security

$module = Join-Path $destination 'tinymcp.dll'
Copy-Item -LiteralPath $source -Destination $module
Write-Output "staged TinyMCP module with a private loader ACL at $module"
