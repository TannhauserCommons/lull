#!/bin/sh
# Every route in budgets.json against its budgets, one lhci run each; fails if any route does.
set -u
cd "$(dirname "$0")"

# Lighthouse slows the CPU 4x on top of whatever runner it is on, and GitHub's runners differ by 2x: in one day of
# queue runs Lighthouse's benchmark of the runner went from 1,900 to 4,000, and /now's blocking time with it, from 30
# to 170 ms for the same code. So the slowdown is set for this runner: 4x times its speed over the speed the budgets
# were met on (budgets.json, benchmarkIndex). Every runner then plays the same device, and what the page itself
# does shows the same on all of them. The runner's speed is Lighthouse's own benchmark, the median of three loads.
base=${LULL_URL:-http://127.0.0.1:8080}
index=$(for i in 1 2 3; do
  npx lighthouse "$base/" --quiet --chrome-flags='--no-sandbox --headless=new' --only-audits=total-blocking-time \
    --output=json --output-path=stdout | node -e 'let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => console.log(JSON.parse(s).environment.benchmarkIndex))'
done | sort -n | sed -n 2p)
LHCI_CPU_SLOWDOWN=$(node -p "const i = +process.argv[1]; if (!(i > 0)) throw new Error('no benchmark of this runner: ' + process.argv[1]); (4 * i / require('./budgets.json').benchmarkIndex).toFixed(2)" "$index") || exit 1
export LHCI_CPU_SLOWDOWN
echo "this runner's benchmark: $index; the CPU slowed ${LHCI_CPU_SLOWDOWN}x"

# LH_SHARD=i/n (CI) takes every n-th route from the i-th, so n machines share the routes.
shard=${LH_SHARD:-1/1}
routes=$(node -p "const [i, n] = process.argv[1].split('/').map(Number); Object.keys(require('./budgets.json').routes).filter((_, k) => k % n === i - 1).join(' ')" "$shard")
status=0
for route in $routes; do
  echo "== $route"
  LHCI_ROUTE=$route npx lhci autorun --config=lighthouserc.cjs || status=1
  node shipped.mjs "$route" || status=1
done
exit $status
