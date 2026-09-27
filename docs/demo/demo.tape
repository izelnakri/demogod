# docs/demo.gif, the README's: `make demo`.
#
# A demo of demogod, made by demogod: it writes a tape, records it, watches the result in the
# browser pane, then drives a page there. hello.tape is the tape it records; site/ the page.

Output ../demo.gif

Set Width 1280
Set Height 640
Set FontSize 16
Set Theme "Catppuccin Mocha"
Set PaneWidth 46%
Set TypingSpeed 40ms

# Off camera: this checkout's demogod on PATH, bat for colours when it is installed, and the
# page the last scene drives already open beside the terminal.
Hide
Type `export PATH="$PWD/../../target/release:$PATH"; command -v bat >/dev/null && alias cat='bat -pp -l bash'; clear`
Enter
Wait
Open "site/index.html"
Focus terminal
Show

Caption "Write a tape" "VHS's commands, plus captions, a browser pane and actions"
Sleep 400ms
Type "cat hello.tape"
Enter
Wait
Sleep 2.5s

Caption "Record it" "a terminal needs nothing installed: no ttyd, no Chrome, no ffmpeg"
Sleep 400ms
Type "demogod hello.tape"
Enter
Wait /done in/
Sleep 1.5s

Caption "Watch it" "tape timing: the same film, however fast or busy the machine"
Open "hello.gif"
Sleep 3.2s

Caption "Drive a browser" "real clicks and keys in Chrome, and a pointer to follow them"
Open "site/index.html"
Sleep 300ms
Click "#new"
Type "Record the demo"
Click "text=Add"
Wait /Record the demo/
Sleep 2s
