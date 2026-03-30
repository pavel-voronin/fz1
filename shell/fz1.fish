# fz1 fish integration
#
# Usage — add to ~/.config/fish/config.fish:
#   source /path/to/fz1/shell/fz1.fish
#
# Default binding: Ctrl+X g

function fz1-widget
    set -l result (fz1 </dev/tty 2>/dev/tty)
    set -l code $status
    if test $code -eq 0 -a -n "$result"
        commandline --insert -- $result
    end
    commandline -f repaint
end

bind \cxg fz1-widget
