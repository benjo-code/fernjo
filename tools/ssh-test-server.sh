#!/usr/bin/env bash
# Throwaway OpenSSH server for the fernjo-ssh integration tests.
#
#   tools/ssh-test-server.sh start   prints `export NAME=value` lines (use: eval "$(... start)")
#   tools/ssh-test-server.sh stop    removes the container and the temporary key directory
#
# Needs docker and ssh-keygen; ssh-agent/ssh-add are optional. Never touches ~/.ssh. Values contain
# no spaces or quotes. The agent (unauthorised key first, authorised Ed25519 key second) is started
# where it can work: not in Git Bash on Windows, where the Rust client talks to the OpenSSH named
# pipe instead of a Unix socket, so FERNJO_TEST_SSH_AGENT and SSH_AUTH_SOCK are not printed there.
set -euo pipefail

# Pinned by digest (resolved with `docker buildx imagetools inspect ...:latest`).
IMAGE="lscr.io/linuxserver/openssh-server@sha256:161e0b670b31d9539ca1718bc5943e981779331b4e1c9d32e8929d7652a1f22f"
NAME="fernjo-ssh-test"
USER_NAME="fernjo"
STATE_DIR="${TMPDIR:-/tmp}/${NAME}-state"

native_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

random_word() {
  # 24 hex characters, no quoting issues.
  head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n'
}

# Wait until sshd answers with its banner, not just until the port is mapped.
wait_for_banner() {
  local i
  for i in $(seq 1 90); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$1" && head -c 4 <&3 | grep -q '^SSH-') 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  echo "ssh test server did not come up" >&2
  exit 1
}

agent_supported() {
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) return 1 ;;
  esac
  command -v ssh-agent >/dev/null 2>&1 && command -v ssh-add >/dev/null 2>&1
}

# Starts a private agent on a socket in the state dir and loads the two keys. The agent's pid is
# kept in a file so `stop` can kill it. Best effort: on any failure no agent variables are printed.
start_agent() {
  local dir="$1" sock="$1/agent.sock" out
  agent_supported || return 1
  out="$(ssh-agent -a "$sock" -s)" || return 1
  printf '%s\n' "$out" | sed -n 's/^SSH_AGENT_PID=\([0-9]*\).*/\1/p' >"$dir/agent.pid"
  if ! SSH_AUTH_SOCK="$sock" ssh-add -q "$dir/unauthorised" "$dir/ed25519" 2>/dev/null; then
    kill_agent
    return 1
  fi
  echo "export FERNJO_TEST_SSH_AGENT=1"
  echo "export SSH_AUTH_SOCK=$sock"
}

kill_agent() {
  if [ -f "$STATE_DIR/agent.pid" ]; then
    kill "$(cat "$STATE_DIR/agent.pid")" >/dev/null 2>&1 || true
  fi
}

start() {
  stop_quiet
  mkdir -p "$STATE_DIR"
  chmod 700 "$STATE_DIR"
  local dir="$STATE_DIR"
  local pass password
  pass="$(random_word)"
  password="$(random_word)"

  ssh-keygen -q -t ed25519 -N "" -C fernjo-test -f "$dir/ed25519"
  ssh-keygen -q -t rsa -b 2048 -N "" -C fernjo-test -f "$dir/rsa"
  ssh-keygen -q -t ed25519 -N "$pass" -C fernjo-test -f "$dir/encrypted"
  ssh-keygen -q -t ed25519 -N "" -C fernjo-test -f "$dir/unauthorised"

  docker run -d --name "$NAME" \
    -e PUID=1000 -e PGID=1000 \
    -e USER_NAME="$USER_NAME" -e USER_PASSWORD="$password" -e PASSWORD_ACCESS=true \
    -e PUBLIC_KEY="$(cat "$dir/ed25519.pub")" \
    -p 127.0.0.1::2222 \
    "$IMAGE" >/dev/null

  local port
  port="$(docker port "$NAME" 2222/tcp | head -n 1 | sed 's/.*://')"

  wait_for_banner "$port"

  # The image takes one public key from PUBLIC_KEY; authorise the others as well.
  cat "$dir/rsa.pub" "$dir/encrypted.pub" |
    docker exec -i "$NAME" sh -c 'cat >> /config/.ssh/authorized_keys'

  # The image ships sshd without PAM in use, so keyboard-interactive has nothing to ask. Switch PAM
  # on with a minimal stack (password check against /etc/shadow) and make sshd reread its config.
  printf '%s\n' \
    'auth required pam_unix.so' \
    'account required pam_unix.so' \
    'session required pam_permit.so' |
    docker exec -i "$NAME" sh -c 'cat > /etc/pam.d/sshd'
  docker exec "$NAME" sh -c 'sed -i "s/^#*UsePAM .*/UsePAM yes/" /config/sshd/sshd_config &&
    kill -HUP "$(cat /config/sshd.pid)"'
  sleep 1
  wait_for_banner "$port"

  echo "export FERNJO_TEST_SSH_ADDR=127.0.0.1:$port"
  echo "export FERNJO_TEST_SSH_USER=$USER_NAME"
  echo "export FERNJO_TEST_SSH_PASSWORD=$password"
  echo "export FERNJO_TEST_SSH_KEY_ED25519=$(native_path "$dir/ed25519")"
  echo "export FERNJO_TEST_SSH_KEY_RSA=$(native_path "$dir/rsa")"
  echo "export FERNJO_TEST_SSH_KEY_ENCRYPTED=$(native_path "$dir/encrypted")"
  echo "export FERNJO_TEST_SSH_KEY_PASSPHRASE=$pass"
  echo "export FERNJO_TEST_SSH_KEY_UNAUTHORISED=$(native_path "$dir/unauthorised")"
  # Not fatal when it fails: the agent test then skips locally (and fails in CI, where it must work).
  start_agent "$dir" || true
}

stop_quiet() {
  kill_agent
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  rm -rf "$STATE_DIR"
}

case "${1:-}" in
  start) start ;;
  stop) stop_quiet ;;
  *)
    echo "usage: $0 start|stop" >&2
    exit 2
    ;;
esac
