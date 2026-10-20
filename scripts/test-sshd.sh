#!/bin/bash
# Testler için geçici, root gerektirmeyen bir sshd başlatır (127.0.0.1:2222) ve
# LIMAN_TEST_SSH değerini yazdırır:
#
#   export LIMAN_TEST_SSH=$(scripts/test-sshd.sh) && (cd src-tauri && cargo test)
#   scripts/test-sshd.sh stop
set -euo pipefail
D=${LIMAN_SSHD_DIR:-/tmp/liman-test-sshd}
if [ "${1:-}" = stop ]; then
  [ -f "$D/sshd.pid" ] && kill "$(cat "$D/sshd.pid")" 2>/dev/null || true
  exit 0
fi
mkdir -p "$D"
[ -f "$D/host_ed25519" ] || ssh-keygen -q -t ed25519 -N "" -f "$D/host_ed25519"
[ -f "$D/client_ed25519" ] || ssh-keygen -q -t ed25519 -N "" -f "$D/client_ed25519"
cp "$D/client_ed25519.pub" "$D/authorized_keys" && chmod 600 "$D/authorized_keys"
SFTP=""
for f in /usr/libexec/sftp-server /usr/lib/openssh/sftp-server /usr/lib/ssh/sftp-server; do
  if [ -x "$f" ]; then SFTP=$f; break; fi
done
cat > "$D/sshd_config" <<CONF
Port 2222
ListenAddress 127.0.0.1
HostKey $D/host_ed25519
AuthorizedKeysFile $D/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PidFile $D/sshd.pid
AllowTcpForwarding yes
Subsystem sftp $SFTP
# Testler paralel bağlanır; varsayılanlar bağlantıları rastgele düşürür.
MaxStartups 100
CONF
# OpenSSH 9.8+: kimlik doğrulamadan kopan bağlantılar (anahtar reddi testi) kaynağı cezalandırmasın.
if /usr/sbin/sshd -T -f "$D/sshd_config" -o PerSourcePenalties=no >/dev/null 2>&1; then
  echo "PerSourcePenalties no" >> "$D/sshd_config"
fi
if ! { [ -f "$D/sshd.pid" ] && kill -0 "$(cat "$D/sshd.pid")" 2>/dev/null; }; then
  /usr/sbin/sshd -f "$D/sshd_config" -E "$D/sshd.log"
  sleep 1
fi
echo "127.0.0.1:2222:$(whoami):$D/client_ed25519"
