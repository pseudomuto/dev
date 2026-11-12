# The session recorded into demo/demo.gif. See demo/Dockerfile.

Set Shell bash
Set FontSize 18
Set Width 1200
Set Height 720
Set Padding 24
Set TypingSpeed 40ms

# The shell setup, off camera, as an rc file would do it.
Hide
Type 'eval "$(dev shell-init)" && clear'
Enter
Show

Type "cat demo/dev.yaml"
Enter
Sleep 5s

Hide
Type "clear"
Enter
Show

Type "cd demo && dev up"
Enter
# Real installs over the network, so allow plenty of time.
Wait@5m
Sleep 3s

Type "jq --version && yq --version"
Enter
Sleep 2.5s

Hide
Type "clear"
Enter
Show

# Separate commands: the env changes when the next prompt is drawn.
Type 'echo "$GREETING"'
Enter
Sleep 2s

Type "cd .."
Enter
Sleep 1s

Type 'echo "${GREETING:-unset}"'
Enter
Sleep 2.5s

Hide
Type "cd demo && clear"
Enter
Show

Type "dev --help"
Enter
Sleep 4s

Type "dev greet Muto --loud"
Enter
Sleep 3s
