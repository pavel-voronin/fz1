# fz1 zsh integration
#
# Usage — add to ~/.zshrc:
#   source /path/to/fz1/shell/fz1.zsh
#
# Default binding: Ctrl+X g

fz1-widget() {
  local result
  result=$(fz1 </dev/tty 2>/dev/tty)
  local code=$?
  if [[ $code -eq 0 && -n $result ]]; then
    LBUFFER+=$result
  fi
  zle reset-prompt
}

zle -N fz1-widget
bindkey '^Xg' fz1-widget
