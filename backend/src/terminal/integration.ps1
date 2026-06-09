# Episteme terminal shell integration (PowerShell). Dot-sourced at startup.
# Wraps the prompt so each command emits OSC 633 markers the app uses for the
# searchable history and for the AI agent's output/exit-code capture. The
# xterm.js client swallows these, so nothing renders. Preserves any existing
# prompt function.
#
#   OSC 633;D;<ec>   — command finished, with its exit code
#   OSC 633;E;<cmd>  — the command line that ran (for history)
# (PowerShell has no pre-execution hook, so there is no 633;C marker; the agent
#  trims the echoed command line instead.)

$global:__EpiInnerPrompt = $function:prompt

function global:prompt {
    $esc = [char]27
    $bel = [char]7
    $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    [Console]::Write("$esc]633;D;$code$bel")

    $last = Get-History -Count 1 -ErrorAction SilentlyContinue
    if ($last -and $last.CommandLine) {
        [Console]::Write("$esc]633;E;$($last.CommandLine)$bel")
    }

    if ($global:__EpiInnerPrompt) {
        & $global:__EpiInnerPrompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
}
