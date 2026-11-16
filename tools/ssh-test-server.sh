#!/usr/bin/env bash
# Throwaway OpenSSH server for the fernjo-ssh integration tests.
#
#   tools/ssh-test-server.sh start   prints `export NAME=value` lines (use: eval "$(... start)")
#   tools/ssh-test-server.sh stop    removes the container and the temporary key directory
#
# Needs docker and ssh-keygen. Never touches ~/.ssh. Values contain no spaces or quotes.
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
}

stop_quiet() {
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
