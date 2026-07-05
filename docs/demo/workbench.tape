Output docs/assets/demo.gif
Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 16
Set LetterSpacing 0
Set LineHeight 1.15
Set Width 1600
Set Height 900
Set Theme "TokyoNight"
Set TypingSpeed 25ms

Hide
Type "unset NO_COLOR"
Enter
Type "export TERM=xterm-256color"
Enter
Type "export COLORTERM=truecolor"
Enter
Type "export ZW_CONFIG_DIR=/tmp/zellij-workbench-vhs-demo/config"
Enter
Type "export ZW_DATA_DIR=/tmp/zellij-workbench-vhs-demo/data"
Enter
Type "rm -rf /tmp/zellij-workbench-vhs-demo && ./scripts/demo-fixture.sh /tmp/zellij-workbench-vhs-demo/config /tmp/zellij-workbench-vhs-demo/data >/dev/null"
Enter
Sleep 1500ms
Type "clear"
Enter
Sleep 500ms
Show

Sleep 500ms
Type "zw"
Enter
Sleep 1800ms
Type "/"
Sleep 200ms
Type "server:prod git:dirty"
Sleep 1800ms
Escape
Sleep 400ms
Type "s"
Sleep 800ms
Type "v"
Sleep 800ms
Type "j"
Sleep 800ms
Type "j"
Sleep 800ms
Type "k"
Sleep 1000ms
Type "q"
Sleep 500ms
