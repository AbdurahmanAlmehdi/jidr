# Terminal beat of the README demo (15–20 s). Runs the real commands against
# the rooted device that is the only one attached, with frida-server running.
# Render: vhs demo/recording/terminal.tape

Output demo/recording/out/terminal.mp4

Set Shell zsh
Set FontFamily "Menlo"
Set FontSize 28
Set Width 1440
Set Height 900
Set Padding 48
Set LineHeight 1.0
Set TypingSpeed 35ms
Set CursorBlink false
Set WindowBar Colorful
Set WindowBarSize 44
Set BorderRadius 12
Set Margin 0
Set Theme { "name": "Jidr", "background": "#0E1014", "foreground": "#E7E9EE", "cursor": "#7AA2FF", "selection": "#20242B", "black": "#0A0C0F", "red": "#F2707A", "green": "#5FD49A", "yellow": "#E8C267", "blue": "#7AA2FF", "magenta": "#B794F6", "cyan": "#6CC7D9", "white": "#E7E9EE", "brightBlack": "#5C6370", "brightRed": "#F2707A", "brightGreen": "#5FD49A", "brightYellow": "#E8C267", "brightBlue": "#7AA2FF", "brightMagenta": "#B794F6", "brightCyan": "#6CC7D9", "brightWhite": "#FFFFFF" }

Hide
# frida-ps pads each row to the terminal width, which wraps; trim the padding.
Type `frida-ps() { command frida-ps "$@" | sed 's/ *$//'; }`
Enter
Type "export PATH=$HOME/.local/bin:/usr/local/bin:/opt/homebrew/bin:$PATH PROMPT='%F{blue}~%f %F{8}$%f ' RPROMPT='' && clear"
Enter
Sleep 500ms
Show

Sleep 400ms
Type "adb shell su -c id"
Sleep 300ms
Enter
Sleep 2s
Type "frida-ps -U"
Sleep 300ms
Enter
Sleep 2.5s
