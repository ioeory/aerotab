"""Controlled real PTY automation, never a human visual or GPU smoke test.
JSONL commands return bounded output bursts. Only a unique private tmux socket is
used; HOME/XDG directories and Vim config/history are isolated and cleaned up.
"""
import base64
import errno
import fcntl
import json
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import uuid

kind = sys.argv[1]
with tempfile.TemporaryDirectory(prefix="handoff-pty-") as home:
    socket = os.path.join(home, "tmux-" + uuid.uuid4().hex)
    env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home, XDG_DATA_HOME=home,
               TERM="xterm-256color", LANG="C.UTF-8", LC_ALL="C.UTF-8", PS1="SPIKE> ",
               HISTFILE="/dev/null", ENV="/dev/null")
    env.pop("TMUX", None)
    pid, master = pty.fork()
    if pid == 0:
        if kind == "vim":
            args = ["vim", "-Nu", "NONE", "-U", "NONE", "-i", "NONE", "-n", "--noplugin",
                    "+set noshowmode noruler laststatus=0 shortmess+=I"]
        else:
            args = ["tmux", "-S", socket, "-f", "/dev/null", "new-session", "-s", "spike", "/bin/sh"]
        os.execvpe(args[0], args, env)
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    def capture():
        chunks = []
        total = 0
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if not select.select([master], [], [], 0.2)[0]:
                if chunks:
                    break
                continue
            try:
                data = os.read(master, 4096)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not data:
                break
            total += len(data)
            if total > 1024 * 1024:
                raise RuntimeError("PTY output burst exceeded 1MiB limit")
            chunks.append(base64.b64encode(data).decode())
        return chunks

    try:
        for line in sys.stdin:
            command = json.loads(line)
            if command.get("input") is not None:
                os.write(master, base64.b64decode(command["input"]))
            result = {"chunks": capture(), "kind": kind}
            print(json.dumps(result), flush=True)
            if command.get("quit"):
                break
    finally:
        if kind == "tmux":
            subprocess.run(["tmux", "-S", socket, "kill-server"], env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3, check=False)
        try:
            os.kill(pid, signal.SIGHUP)
        except ProcessLookupError:
            pass
        os.close(master)
        # Reap or force-kill only our child, never a user's terminal/session.
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            done, _ = os.waitpid(pid, os.WNOHANG)
            if done:
                break
            select.select([], [], [], 0.02)
        else:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
