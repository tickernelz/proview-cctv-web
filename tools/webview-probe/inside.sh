#!/bin/sh
root=$1
page=$2
python3 /p/serve.py "$root" 8080 &
server=$!
Xvfb -displayfd 3 -nolisten tcp 3> /tmp/display 2> /dev/null &
while [ ! -s /tmp/display ]; do sleep 0.05; done
DISPLAY=:$(cat /tmp/display) python3 -u /p/gtk_probe.py "http://127.0.0.1:8080/$page" >&2 2> /dev/null &
wait "$server"
