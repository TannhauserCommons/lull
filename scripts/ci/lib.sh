# Shared by the scripts in this folder: sourced after `cd` to the repository root, never run. POSIX sh, so it also runs
# in Git's sh on Windows. The scripts are what the `mise run` tasks of .github/workflows/check.yml call, and what an
# agent runs locally to get the same verdict (docs/context/quality.md, Check in shadow).

# Tauri's documented Linux build prerequisites (WebKitGTK 4.1, AppIndicator, rsvg for icons, xdo), and ALSA's headers for
# native Classic's sound (cpal; libopus builds with the runner's CMake).
LINUX_DEPS="libwebkit2gtk-4.1-dev build-essential file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev"

# Where a script leaves what a later step reads: CI's temp folder, else the machine's.
tmp_root() { printf '%s\n' "${RUNNER_TEMP:-${TMPDIR:-/tmp}}"; }

# The NDK the Android project is built with: the one source of it, which scripts/android-build.sh also checks.
ndk_version() { sed -n 's/.*ndkVersion = "\(.*\)".*/\1/p' app/gen/android/app/build.gradle.kts; }

# Installs the Debian packages that are missing. CI (and root, in a container) installs them; on a workstation it names
# the command instead of asking for a password. Another system gets the list to install by hand.
apt_install() {
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "ci: no apt-get here; the equivalents of these are needed: $*" >&2
    return 0
  fi
  missing=
  for pkg in "$@"; do
    dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q 'install ok installed' || missing="$missing $pkg"
  done
  [ -n "$missing" ] || return 0
  if [ "$(id -u)" = 0 ]; then
    sudo=
  elif [ "${CI:-}" = true ]; then
    sudo=sudo
  else
    echo "ci: install first:  sudo apt-get install -y$missing" >&2
    return 1
  fi
  # shellcheck disable=SC2086 # one word per package, on purpose
  $sudo apt-get update -qq && $sudo apt-get install -y -qq $missing
}

# Scripts that rewrite the checkout (a throwaway signing key, no Gradle wrapper, the web UI swapped for its minified
# build) run on CI's own copy of it.
only_on_ci() { [ "${CI:-}" = true ] || { echo "ci: $1 edits the checkout, so it runs on CI only (set CI=true to run it on a copy you can throw away)" >&2; exit 1; }; }
