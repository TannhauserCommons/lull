#!/bin/sh
# Installs lull from the .tar.gz on a Linux without .deb or .rpm (ADR-0090): the same files and service as the
# packages. Run it as root from the unpacked folder: sudo ./install.sh. Again with a newer one to update by hand.
set -e
here=$(cd "$(dirname "$0")" && pwd)
[ "$(id -u)" = 0 ] || { echo "Run it as root: sudo $0" >&2; exit 1; }
[ -d /run/systemd/system ] || { echo "lull's service needs systemd; run /usr/lib/lull-server/lull serve yourself instead." >&2; }
# slskd is .NET, which aborts at start without ICU (the .deb and .rpm depend on it).
ldconfig -p 2>/dev/null | grep -q libicuuc || echo "Install ICU too (libicu, or libicu<version> on Debian and Ubuntu): Soulseek won't start without it." >&2
mkdir -p /usr/lib/lull-server /usr/share/doc/lull-server
cp -R "$here/lib/." /usr/lib/lull-server/
cp -R "$here/licenses/." /usr/share/doc/lull-server/
ln -sf /usr/lib/lull-server/lull /usr/bin/lull
cp "$here/lull.service" /etc/systemd/system/lull.service
sh "$here/postinstall.sh"
