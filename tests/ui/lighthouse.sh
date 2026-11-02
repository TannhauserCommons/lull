#!/bin/sh
# Every route in budgets.json against its budgets, one lhci run each; fails if any route does.
set -u
cd "$(dirname "$0")"
status=0
for route in $(node -p "Object.keys(require('./budgets.json').routes).join(' ')"); do
  echo "== $route"
  LHCI_ROUTE=$route npx lhci autorun --config=lighthouserc.cjs || status=1
done
exit $status
