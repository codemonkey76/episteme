# Episteme terminal shell integration (bash). Loaded via `bash --rcfile`.
# Keeps the user's normal environment, then emits OSC 633 markers so the app
# can (a) record a searchable command history and (b) let the AI agent capture
# each command's output and exit code. The xterm.js client swallows these
# sequences, so nothing renders. None of this changes how commands run.
#
#   OSC 633;C        — command output begins (PS0, after the line is read)
#   OSC 633;D;<ec>   — command finished, with its exit code
#   OSC 633;E;<cmd>  — the command line that ran (for history)

# Restore the standard interactive environment first.
if [ -f /etc/bash.bashrc ]; then . /etc/bash.bashrc; fi
if [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi

__epi_report_command() {
    # Capture the just-finished command's exit code FIRST (before anything else
    # touches $?), then report it and the command line.
    local ec=$?
    printf '\033]633;D;%s\007' "$ec"
    local cmd
    cmd=$(HISTTIMEFORMAT= history 1 2>/dev/null | sed 's/^ *[0-9][0-9]* *//')
    [ -n "$cmd" ] && printf '\033]633;E;%s\007' "$cmd"
}

# Run our reporter before any pre-existing PROMPT_COMMAND so $? is still ours.
if [ -n "$PROMPT_COMMAND" ]; then
    PROMPT_COMMAND="__epi_report_command; $PROMPT_COMMAND"
else
    PROMPT_COMMAND="__epi_report_command"
fi

# PS0 is printed after a command line is read but before it runs — the exact
# boundary where command output starts.
PS0=$'\033]633;C\007'
