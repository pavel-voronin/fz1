# fz1 bash integration
#
# Usage — add to ~/.bashrc:
#   source <(fz1 integration bash)
#
# Default binding: Ctrl+X g
# Requires bash 4+

fz1_widget() {
  local result
  result=$(fz1 </dev/tty 2>/dev/tty)
  local code=$?
  if [[ $code -eq 0 && -n $result ]]; then
    READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}${result}${READLINE_LINE:READLINE_POINT}"
    READLINE_POINT=$((READLINE_POINT + ${#result}))
  fi
}

bind -x '"\C-xg":fz1_widget'
