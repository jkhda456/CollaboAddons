#!/usr/bin/env bash
# Starts a throwaway OpenSSH sshd on a free port for the SSH client tests, with a known host key
# and a user key we authorize. Prints "PORT HOSTKEYFILE USERKEY DIR" and backgrounds sshd.
set -euo pipefail
D="$(mktemp -d)"
ssh-keygen -q -t ed25519 -f "$D/hostkey" -N "" -C host
ssh-keygen -q -t ed25519 -f "$D/userkey" -N "" -C user
mkdir -p "$D/home/.ssh"
cp "$D/userkey.pub" "$D/home/.ssh/authorized_keys"
chmod 700 "$D/home/.ssh"; chmod 600 "$D/home/.ssh/authorized_keys"
USER_NAME="$(id -un)"
cat > "$D/sshd_config" <<CFG
Port 0
ListenAddress 127.0.0.1
HostKey $D/hostkey
PidFile $D/sshd.pid
LogLevel QUIET
UsePAM no
StrictModes no
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
AuthorizedKeysFile $D/home/.ssh/authorized_keys
Subsystem sftp internal-sftp
AcceptEnv CSH_TEST
PermitUserEnvironment no
CFG
# Optional algorithm restrictions, to test one cipher/mac/kex at a time.
[ -n "${CSH_CIPHERS:-}" ] && echo "Ciphers $CSH_CIPHERS" >> "$D/sshd_config"
[ -n "${CSH_MACS:-}" ] && echo "MACs $CSH_MACS" >> "$D/sshd_config"
[ -n "${CSH_KEX:-}" ] && echo "KexAlgorithms $CSH_KEX" >> "$D/sshd_config"
# A password login option (for the password test): enable it and set it via chpasswd-free means
# is not possible here, so password auth is tested separately with a helper account when asked.
if [ -n "${CSH_PASSWORD_AUTH:-}" ]; then
  sed -i 's/^PasswordAuthentication no/PasswordAuthentication yes/' "$D/sshd_config"
  sed -i 's/^KbdInteractiveAuthentication no/KbdInteractiveAuthentication yes/' "$D/sshd_config"
fi
# A free port.
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
sed -i "s/^Port 0/Port $PORT/" "$D/sshd_config"
/usr/sbin/sshd -f "$D/sshd_config" -o "PidFile=$D/sshd.pid" &
for _ in $(seq 1 50); do
  if python3 -c "import socket;socket.create_connection(('127.0.0.1',$PORT),0.2).close()" 2>/dev/null; then break; fi
  sleep 0.1
done
echo "$PORT $D/hostkey.pub $D/userkey $USER_NAME $D"
