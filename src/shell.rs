/// Shell integration snippets for eval in .zshrc / .bashrc.
///
/// The `auto_embed` parameter adds `--embed` to the record call so each
/// command is embedded inline (Ollama must be reachable; failures are silent).

pub fn zsh_snippet(auto_embed: bool) -> String {
    let embed_flag = if auto_embed { " \\\n            --embed" } else { "" };
    format!(r#"
# --- tapeworm zsh integration ---
# Install: add `eval "$(tapeworm init --shell zsh)"` to ~/.zshrc

export TAPEWORM_SESSION="$(tapeworm session-id)"

_tw_start=0
_tw_cmd=""
_tw_gap=0
_tw_prev_end=0   # epoch ms when the last precmd ran (= last command finished)

function _tapeworm_preexec() {{
    _tw_cmd="$1"
    _tw_start=$(date +%s%3N)
    # Gap = idle + think time since the last command finished
    if [[ "$_tw_prev_end" -gt 0 ]]; then
        _tw_gap=$(( _tw_start - _tw_prev_end ))
    else
        _tw_gap=0
    fi
}}

function _tapeworm_precmd() {{
    local _tw_exit=$?
    local _tw_end
    _tw_end=$(date +%s%3N)
    local _tw_duration=$(( _tw_end - _tw_start ))

    if [[ -n "$_tw_cmd" ]]; then
        tapeworm record \
            --cmd      "$_tw_cmd" \
            --cwd      "$PWD" \
            --exit     "$_tw_exit" \
            --duration "$_tw_duration" \
            --gap      "$_tw_gap" \
            --session  "$TAPEWORM_SESSION"{embed_flag} \
            &>/dev/null &!
        _tw_cmd=""
        _tw_start=0
        _tw_gap=0
    fi
    _tw_prev_end=$_tw_end
}}

autoload -Uz add-zsh-hook
add-zsh-hook preexec _tapeworm_preexec
add-zsh-hook precmd  _tapeworm_precmd
# --- end tapeworm ---
"#, embed_flag = embed_flag)
}

pub fn bash_snippet(auto_embed: bool) -> String {
    let embed_flag = if auto_embed { " \\\n            --embed" } else { "" };
    format!(r#"
# --- tapeworm bash integration ---
# Install: add `eval "$(tapeworm init --shell bash)"` to ~/.bashrc

export TAPEWORM_SESSION="$(tapeworm session-id)"

_tw_start=0
_tw_cmd=""
_tw_gap=0
_tw_prev_end=0
_tw_in_prompt=0

_tapeworm_debug() {{
    # Guard against self-recursion from PROMPT_COMMAND
    if [[ "$_tw_in_prompt" == "0" && "$BASH_COMMAND" != "_tapeworm_precmd" ]]; then
        _tw_cmd="$BASH_COMMAND"
        _tw_start=$(date +%s%3N)
        if [[ "$_tw_prev_end" -gt 0 ]]; then
            _tw_gap=$(( _tw_start - _tw_prev_end ))
        else
            _tw_gap=0
        fi
    fi
}}

_tapeworm_precmd() {{
    local _tw_exit=$?
    _tw_in_prompt=1
    local _tw_end
    _tw_end=$(date +%s%3N)
    local _tw_duration=$(( _tw_end - _tw_start ))

    if [[ -n "$_tw_cmd" ]]; then
        tapeworm record \
            --cmd      "$_tw_cmd" \
            --cwd      "$PWD" \
            --exit     "$_tw_exit" \
            --duration "$_tw_duration" \
            --gap      "$_tw_gap" \
            --session  "$TAPEWORM_SESSION"{embed_flag} \
            &>/dev/null &
        _tw_cmd=""
        _tw_start=0
        _tw_gap=0
    fi
    _tw_prev_end=$_tw_end
    _tw_in_prompt=0
}}

trap '_tapeworm_debug' DEBUG
PROMPT_COMMAND="_tapeworm_precmd${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}"
# --- end tapeworm ---
"#, embed_flag = embed_flag)
}
