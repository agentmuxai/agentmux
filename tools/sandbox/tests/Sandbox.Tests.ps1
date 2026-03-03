# Pester tests for @agentmuxhq/sandbox package

BeforeAll {
    $ScriptRoot = Split-Path -Parent $PSScriptRoot
    $ScriptsDir = Join-Path $ScriptRoot "scripts"
    $ConfigDir = Join-Path $ScriptRoot "config"
    $BinDir = Join-Path $ScriptRoot "bin"
}

Describe "Package Structure" {
    It "Should have scripts directory" {
        Test-Path $ScriptsDir | Should -Be $true
    }

    It "Should have config directory" {
        Test-Path $ConfigDir | Should -Be $true
    }

    It "Should have bin directory" {
        Test-Path $BinDir | Should -Be $true
    }

    It "Should have package.json" {
        Test-Path (Join-Path $ScriptRoot "package.json") | Should -Be $true
    }

    It "Should have README.md" {
        Test-Path (Join-Path $ScriptRoot "README.md") | Should -Be $true
    }
}

Describe "Scripts Exist" {
    It "Should have setup-sandbox-impl.ps1" {
        Test-Path (Join-Path $ScriptsDir "setup-sandbox-impl.ps1") | Should -Be $true
    }

    It "Should have install-dev-tools.ps1" {
        Test-Path (Join-Path $ScriptsDir "install-dev-tools.ps1") | Should -Be $true
    }

    It "Should have install-parsec.ps1" {
        Test-Path (Join-Path $ScriptsDir "install-parsec.ps1") | Should -Be $true
    }

    It "Should have clone-wavemux.ps1" {
        Test-Path (Join-Path $ScriptsDir "clone-wavemux.ps1") | Should -Be $true
    }

    It "Should have sandbox-health-impl.ps1" {
        Test-Path (Join-Path $ScriptsDir "sandbox-health-impl.ps1") | Should -Be $true
    }
}

Describe "Bin Wrappers Exist" {
    It "Should have setup-sandbox-host.ps1" {
        Test-Path (Join-Path $BinDir "setup-sandbox-host.ps1") | Should -Be $true
    }

    It "Should have sandbox-health.ps1" {
        Test-Path (Join-Path $BinDir "sandbox-health.ps1") | Should -Be $true
    }
}

Describe "Config Files" {
    It "Should have parsec-config.json" {
        Test-Path (Join-Path $ConfigDir "parsec-config.json") | Should -Be $true
    }

    It "Should have wavemux-instance.json" {
        Test-Path (Join-Path $ConfigDir "wavemux-instance.json") | Should -Be $true
    }

    It "parsec-config.json should be valid JSON" {
        $ConfigPath = Join-Path $ConfigDir "parsec-config.json"
        { Get-Content $ConfigPath -Raw | ConvertFrom-Json } | Should -Not -Throw
    }

    It "wavemux-instance.json should be valid JSON" {
        $ConfigPath = Join-Path $ConfigDir "wavemux-instance.json"
        { Get-Content $ConfigPath -Raw | ConvertFrom-Json } | Should -Not -Throw
    }

    It "parsec-config.json should have headless settings" {
        $ConfigPath = Join-Path $ConfigDir "parsec-config.json"
        $Config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
        $Config.host_virtual_monitor | Should -Be 1
        $Config.host_virtual_monitor_fallback | Should -Be 1
    }

    It "wavemux-instance.json should specify dev instance" {
        $ConfigPath = Join-Path $ConfigDir "wavemux-instance.json"
        $Config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
        $Config.instance | Should -Be "dev"
    }
}

Describe "Script Syntax" {
    It "setup-sandbox-impl.ps1 should have valid PowerShell syntax" {
        $ScriptPath = Join-Path $ScriptsDir "setup-sandbox-impl.ps1"
        $Errors = $null
        $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content $ScriptPath -Raw), [ref]$Errors)
        $Errors.Count | Should -Be 0
    }

    It "install-dev-tools.ps1 should have valid PowerShell syntax" {
        $ScriptPath = Join-Path $ScriptsDir "install-dev-tools.ps1"
        $Errors = $null
        $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content $ScriptPath -Raw), [ref]$Errors)
        $Errors.Count | Should -Be 0
    }

    It "install-parsec.ps1 should have valid PowerShell syntax" {
        $ScriptPath = Join-Path $ScriptsDir "install-parsec.ps1"
        $Errors = $null
        $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content $ScriptPath -Raw), [ref]$Errors)
        $Errors.Count | Should -Be 0
    }

    It "clone-wavemux.ps1 should have valid PowerShell syntax" {
        $ScriptPath = Join-Path $ScriptsDir "clone-wavemux.ps1"
        $Errors = $null
        $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content $ScriptPath -Raw), [ref]$Errors)
        $Errors.Count | Should -Be 0
    }

    It "sandbox-health-impl.ps1 should have valid PowerShell syntax" {
        $ScriptPath = Join-Path $ScriptsDir "sandbox-health-impl.ps1"
        $Errors = $null
        $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content $ScriptPath -Raw), [ref]$Errors)
        $Errors.Count | Should -Be 0
    }
}

Describe "Script Help" {
    It "setup-sandbox-impl.ps1 should have comment-based help" {
        $ScriptPath = Join-Path $ScriptsDir "setup-sandbox-impl.ps1"
        $Content = Get-Content $ScriptPath -Raw
        $Content | Should -Match '\.SYNOPSIS'
        $Content | Should -Match '\.DESCRIPTION'
    }

    It "install-dev-tools.ps1 should have comment-based help" {
        $ScriptPath = Join-Path $ScriptsDir "install-dev-tools.ps1"
        $Content = Get-Content $ScriptPath -Raw
        $Content | Should -Match '\.SYNOPSIS'
        $Content | Should -Match '\.DESCRIPTION'
    }

    It "sandbox-health-impl.ps1 should have comment-based help" {
        $ScriptPath = Join-Path $ScriptsDir "sandbox-health-impl.ps1"
        $Content = Get-Content $ScriptPath -Raw
        $Content | Should -Match '\.SYNOPSIS'
        $Content | Should -Match '\.DESCRIPTION'
    }
}

Describe "Health Check Output" {
    It "sandbox-health-impl.ps1 should support JSON output" {
        $ScriptPath = Join-Path $ScriptsDir "sandbox-health-impl.ps1"
        $Content = Get-Content $ScriptPath -Raw
        $Content | Should -Match 'OutputFormat'
        $Content | Should -Match 'ConvertTo-Json'
    }
}
