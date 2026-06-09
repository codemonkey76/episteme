# Episteme terminal shell integration (PowerShell). Dot-sourced at startup.
# Wraps the prompt so each command the user runs is reported to the app as an
# OSC 633;E sequence, which the xterm.js client parses and swallows for the
# searchable history. Preserves any existing prompt function.

$global:__EpiInnerPrompt = $function:prompt

function global:prompt {
    $last = Get-History -Count 1 -ErrorAction SilentlyContinue
    if ($last -and $last.CommandLine) {
        $esc = [char]27
        $bel = [char]7
        [Console]::Write("$esc]633;E;$($last.CommandLine)$bel")
    }
    if ($global:__EpiInnerPrompt) {
        & $global:__EpiInnerPrompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
}
