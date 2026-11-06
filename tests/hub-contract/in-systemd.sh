#!/bin/sh
# The single-quoted commands expand inside the container, on purpose.
# shellcheck disable=SC2016
# Installs a Linux server package (ADR-0090) in a fresh container of <distro> running systemd, as on a real
# server, and runs the hub contract against its service: installed, started, set to start at boot, its own slskd
# and YouTube programs running, `lull pair` pairing, answering on mDNS. Then removes it and checks it stopped.
#
#   sh tests/hub-contract/in-systemd.sh ubuntu|debian|fedora <.deb or .rpm>
set -eu
distro=$1 package=$2
# Git Bash on Windows hands docker a Windows path only from pwd -W.
here=$(cd "$(dirname "$0")" && (pwd -W 2>/dev/null || pwd))
case "$distro" in
  ubuntu) from=ubuntu:24.04 setup="apt-get update -qq && apt-get install -y -qq systemd systemd-sysv nodejs procps ca-certificates" install="apt-get install -y -qq" ;;
  debian) from=debian:trixie setup="apt-get update -qq && apt-get install -y -qq systemd systemd-sysv nodejs procps ca-certificates" install="apt-get install -y -qq" ;;
  fedora) from=fedora:43 setup="dnf -y -q install systemd nodejs procps-ng util-linux" install="dnf -y -q install" ;;
  *) echo "no $distro" >&2; exit 1 ;;
esac
image=lull-systemd-$distro
printf 'FROM %s\nRUN %s\nCMD ["/sbin/init"]\n' "$from" "$setup" | docker build -q -t "$image" - >/dev/null
box=$(docker run -d --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock "$image")
trap 'docker rm -f "$box" >/dev/null' EXIT
inside() { docker exec "$box" sh -c "$1"; }
until inside 'systemctl is-system-running 2>/dev/null | grep -Eq "running|degraded"'; do sleep 1; done
docker cp "$package" "$box:/root/$(basename "$package")"
docker cp "$here/run.mjs" "$box:/root/run.mjs"

echo "== install $(basename "$package") on $from"
inside "DEBIAN_FRONTEND=noninteractive $install /root/$(basename "$package")"
inside 'systemctl is-enabled lull && systemctl is-active lull'
inside 'for i in $(seq 1 60); do [ -S /var/lib/lull/admin.sock ] && exit 0; sleep 1; done; journalctl -u lull --no-pager | tail -20; exit 1'
inside 'test "$(stat -c %U /var/lib/lull)" = lull'
inside 'pgrep -u lull -x slskd >/dev/null' || { inside 'journalctl -u lull --no-pager | tail -20'; echo "slskd doesn't run as lull" >&2; exit 1; }
inside 'runuser -u lull -- /usr/lib/lull/deno --version | head -1'
code=$(inside 'lull pair' | sed -n 's/.*enter the code \([A-Z0-9]*\).*/\1/p')
[ -n "$code" ] || { echo "lull pair printed no code" >&2; exit 1; }
inside "node /root/run.mjs http://127.0.0.1:8484 --pair $code --managed --mdns"
inside 'lull status'

echo "== remove"
case "$distro" in fedora) inside 'dnf -y -q remove lull' ;; *) inside 'DEBIAN_FRONTEND=noninteractive apt-get remove -y -qq lull' ;; esac
inside '! systemctl is-active lull && ! pgrep -x slskd && test -d /var/lib/lull'
echo "== $distro: the package keeps the contract"
