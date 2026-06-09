# Episteme terminal shell integration (bash). Loaded via `bash --rcfile`.
# Keep the user's normal environment, then add a precmd hook that reports the
# last-run command to the app as an OSC 633;E sequence. The xterm.js client
# parses and swallows it (it never renders), recording the command to the
# searchable history. Nothing here changes how commands run.

# Restore the standard interactive environment first.
if [ -f /etc/bash.bashrc ]; then . /etc/bash.bashrc; fi
if [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi

__epi_report_command() {
    # The just-finished command line, history-number prefix stripped.
    local cmd
    cmd=$(HISTTIMEFORMAT= history 1 2>/dev/null | sed 's/^ *[0-9][0-9]* *//')
    [ -n "$cmd" ] && printf '\033]633;E;%s\007' "$cmd"
}

# Run our reporter before any pre-existing PROMPT_COMMAND.
if [ -n "$PROMPT_COMMAND" ]; then
    PROMPT_COMMAND="__epi_report_command; $PROMPT_COMMAND"
else
    PROMPT_COMMAND="__epi_report_command"
fi
