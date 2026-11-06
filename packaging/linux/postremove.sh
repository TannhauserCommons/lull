#!/bin/sh
# After removal: systemd forgets the unit. The data in /var/lib/lull stays; delete it to start over.
set -e
if [ -d /run/systemd/system ]; then systemctl daemon-reload || true; fi
