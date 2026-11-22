#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Offline unit tests for packaging/vulkan-icd.hook under POSIX sh -e (and
# dash -e when available, matching AppRun.sh's bundled dash).
#
# Runs completely in a temporary directory; never touches real user files.
# Usage: verification/check-vulkan-icd-hook.sh [path/to/dash]
set -euo pipefail
cd "$(dirname "$0")/.."

hook="$PWD/packaging/vulkan-icd.hook"
[[ -f "$hook" ]] || { echo "missing $hook" >&2; exit 1; }

shells=(sh)
if [[ -n "${1:-}" ]]; then
  shells+=("$1")
elif command -v dash >/dev/null 2>&1; then
  shells+=(dash)
else
  # If an AppImage is mounted or built in dist/, we can also test with its
  # bundled dash if desired; sh always runs.
  :
fi

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

# Minimal 20-byte ELF headers matching packaging/vulkan-icd.hook's ELF check:
# magic(4) + class(1) + data(1) + ver/pad(10) + type(2) + machine(2).
make_elf() {
  local path="$1" class="$2" mach="$3"
  mkdir -p "$(dirname "$path")"
  # shellcheck disable=SC2059
  printf "\x7fELF${class}\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x03\x00${mach}" > "$path"
}

make_elf_x86_64() { make_elf "$1" '\x02' '\x3e\x00'; }
make_elf_i386()   { make_elf "$1" '\x01' '\x03\x00'; }
make_elf_arm64()  { make_elf "$1" '\x02' '\xb7\x00'; }

pass=0
fail() { echo "FAIL: $*" >&2; exit 1; }
assert_eq() {
  local got="$1" want="$2" msg="$3"
  [[ "$got" == "$want" ]] || fail "$msg: got=$(printf '%q' "$got") want=$(printf '%q' "$want")"
  pass=$((pass + 1))
}

run_hook() {
  local sh_bin="$1"
  shift
  # Run inside a clean env simulating AppRun.sh (`set -e`, positional args preserved).
  env -i PATH="$PATH" _GLC_VK_SYSCONFDIR="$work/empty-etc" "$@" \
    "$sh_bin" -e -c '
      set -- "--preview" "apps"
      . "$HOOK"
      printf "VK=%s\n" "${VK_DRIVER_FILES:-}"
      printf "HOOK_VK=%s\n" "${GEARLEVER_COSMIC_HOOK_VK_DRIVER_FILES:-}"
      printf "ARGS=%s|%s\n" "$1" "$2"
      if command -v _glc_vk_main >/dev/null 2>&1; then
        printf "LEAK=fn\n"
      else
        printf "LEAK=none\n"
      fi
    '
}

val_of() {
  local out="$1" key="$2"
  printf '%s\n' "$out" | awk -F= -v k="$key" '$1 == k { sub(/^[^=]*=/, ""); print; exit }'
}

# Build fake APPDIR (x86_64) + fake library trees.
appdir="$work/AppDir"
make_elf_x86_64 "$appdir/shared/bin/gearlever-cosmic"
make_elf_x86_64 "$appdir/lib/libvulkan_radeon.so"     # decoy inside APPDIR (must be skipped)
make_elf_i386   "$work/lib32/libvulkan_radeon.so"     # 32-bit decoy (must be skipped)
make_elf_arm64  "$work/libarm/libvulkan_radeon.so"    # foreign-arch decoy (must be skipped)
make_elf_x86_64 "$work/lib64/libvulkan_radeon.so"     # real 64-bit host driver
make_elf_x86_64 "$work/lib64/libvulkan_lvp.so"
make_elf_x86_64 "$work/lib64/libGLX_nvidia.so.0"
make_elf_x86_64 "$work/lib64/libvulkan_mini.so"
make_elf_i386   "$work/ldcfg32/libvulkan_ldcfg_only.so"
make_elf_x86_64 "$work/ldcfg64/libvulkan_ldcfg_only.so"

# Fake ldconfig script for testing the ldconfig -p fallback.
mkdir -p "$work/fakebin"
cat > "$work/fakebin/ldconfig" <<EOF
#!/bin/sh
if [ "\$1" = "-p" ]; then
  printf '\tlibvulkan_ldcfg_only.so (libc6) => %s\n' "$work/ldcfg32/libvulkan_ldcfg_only.so"
  printf '\tlibvulkan_ldcfg_only.so (libc6,x86-64) => %s\n' "$work/ldcfg64/libvulkan_ldcfg_only.so"
fi
EOF
chmod +x "$work/fakebin/ldconfig"

for sh_bin in "${shells[@]}"; do
  echo "== testing with $sh_bin =="

  # 1. Arch/CachyOS bare SONAMEs (no trailing newline), decoys earlier in LD_LIBRARY_PATH.
  t1="$work/t1-$sh_bin"
  mkdir -p "$t1/share/vulkan/icd.d" "$t1/run" "$t1/home"
  printf '{"ICD":{"api_version":"1.4.363","library_path":"libvulkan_lvp.so"},"file_format_version":"1.0.1"}' \
    > "$t1/share/vulkan/icd.d/lvp_icd.json"
  printf '{\n    "ICD": {\n        "api_version": "1.4.363",\n        "library_path": "libvulkan_radeon.so"\n    },\n    "file_format_version": "1.0.1"\n}' \
    > "$t1/share/vulkan/icd.d/radeon_icd.json"

  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t1/home" \
    XDG_RUNTIME_DIR="$t1/run" XDG_DATA_DIRS="$t1/share:$t1/share/" \
    LD_LIBRARY_PATH="$appdir/lib:$work/lib32:$work/libarm:$work/lib64")
  vk=$(val_of "$out" VK)
  hvk=$(val_of "$out" HOOK_VK)
  want="$t1/run/gearlever-cosmic-vk-icd/lvp_icd.json:$t1/run/gearlever-cosmic-vk-icd/radeon_icd.json"
  assert_eq "$vk" "$want" "$sh_bin case1 VK_DRIVER_FILES"
  assert_eq "$hvk" "$want" "$sh_bin case1 HOOK_VK"
  assert_eq "$(val_of "$out" ARGS)" "--preview|apps" "$sh_bin case1 positional args preserved"
  assert_eq "$(val_of "$out" LEAK)" "none" "$sh_bin case1 no leaked functions"
  got_radeon=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t1/run/gearlever-cosmic-vk-icd/radeon_icd.json")
  got_lvp=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t1/run/gearlever-cosmic-vk-icd/lvp_icd.json")
  assert_eq "$got_radeon" "$work/lib64/libvulkan_radeon.so" "$sh_bin case1 radeon library_path"
  assert_eq "$got_lvp" "$work/lib64/libvulkan_lvp.so" "$sh_bin case1 lvp library_path"

  # 2. Already-absolute manifests only -> strict no-op, no cache dir created.
  t2="$work/t2-$sh_bin"
  mkdir -p "$t2/share/vulkan/icd.d" "$t2/run" "$t2/home"
  cat > "$t2/share/vulkan/icd.d/radeon_icd.x86_64.json" <<'EOF'
{
    "ICD": {
        "api_version": "1.4.305",
        "library_path": "/usr/lib64/libvulkan_radeon.so"
    },
    "file_format_version": "1.0.1"
}
EOF
  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t2/home" \
    XDG_RUNTIME_DIR="$t2/run" XDG_DATA_DIRS="$t2/share" \
    LD_LIBRARY_PATH="$work/lib64")
  assert_eq "$(val_of "$out" VK)" "" "$sh_bin case2 VK unset"
  assert_eq "$(val_of "$out" HOOK_VK)" "" "$sh_bin case2 HOOK_VK unset"
  [[ ! -e "$t2/run/gearlever-cosmic-vk-icd" ]] || fail "$sh_bin case2 created cache dir on no-op"
  pass=$((pass + 1))

  # 3. User overrides respected (VK_DRIVER_FILES, VK_ICD_FILENAMES, CROSS_LIBC_DLOPEN=0).
  t3="$work/t3-$sh_bin"
  mkdir -p "$t3/run" "$t3/home"
  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t3/home" \
    XDG_RUNTIME_DIR="$t3/run" XDG_DATA_DIRS="$t1/share" \
    LD_LIBRARY_PATH="$work/lib64" VK_DRIVER_FILES="/custom/user.json")
  assert_eq "$(val_of "$out" VK)" "/custom/user.json" "$sh_bin case3a VK_DRIVER_FILES preserved"
  assert_eq "$(val_of "$out" HOOK_VK)" "" "$sh_bin case3a HOOK_VK unset"

  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t3/home" \
    XDG_RUNTIME_DIR="$t3/run" XDG_DATA_DIRS="$t1/share" \
    LD_LIBRARY_PATH="$work/lib64" VK_ICD_FILENAMES="/custom/legacy.json")
  assert_eq "$(val_of "$out" VK)" "" "$sh_bin case3b VK unset when VK_ICD_FILENAMES set"

  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t3/home" \
    XDG_RUNTIME_DIR="$t3/run" XDG_DATA_DIRS="$t1/share" \
    LD_LIBRARY_PATH="$work/lib64" CROSS_LIBC_DLOPEN=0)
  assert_eq "$(val_of "$out" VK)" "" "$sh_bin case3c VK unset when CROSS_LIBC_DLOPEN=0"
  [[ ! -e "$t3/run/gearlever-cosmic-vk-icd" ]] || fail "$sh_bin case3 created cache dir when skipped"
  pass=$((pass + 1))

  # 4. Mixed manifests: NVIDIA spaced colon, minified single-line, absolute, relative ./, and 32-bit.
  t4="$work/t4-$sh_bin"
  mkdir -p "$t4/share/vulkan/icd.d" "$t4/run" "$t4/home"
  cat > "$t4/share/vulkan/icd.d/10_nvidia_icd.json" <<'EOF'
{
    "file_format_version" : "1.0.0",
    "ICD": {
        "library_path" : "libGLX_nvidia.so.0",
        "api_version" : "1.3.289"
    }
}
EOF
  printf '{"file_format_version":"1.0.0","ICD":{"library_path":"libvulkan_mini.so","api_version":"1.4.0"}}' \
    > "$t4/share/vulkan/icd.d/20_mini_icd.json"
  cat > "$t4/share/vulkan/icd.d/30_abs_icd.json" <<'EOF'
{"ICD":{"library_path":"/usr/lib64/libvulkan_intel.so"}}
EOF
  cat > "$t4/share/vulkan/icd.d/40_rel_icd.json" <<'EOF'
{"ICD":{"library_path":"./libvulkan_rel.so"}}
EOF
  cat > "$t4/share/vulkan/icd.d/50_i686_icd.json" <<'EOF'
{"ICD":{"library_arch":"32","library_path":"libvulkan_radeon.so"}}
EOF
  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t4/home" \
    XDG_RUNTIME_DIR="$t4/run" XDG_DATA_DIRS="$t4/share" \
    LD_LIBRARY_PATH="$work/lib64")
  want4="$t4/run/gearlever-cosmic-vk-icd/10_nvidia_icd.json"
  want4="$want4:$t4/run/gearlever-cosmic-vk-icd/20_mini_icd.json"
  want4="$want4:$t4/share/vulkan/icd.d/30_abs_icd.json"
  want4="$want4:$t4/share/vulkan/icd.d/40_rel_icd.json"
  want4="$want4:$t4/share/vulkan/icd.d/50_i686_icd.json"
  assert_eq "$(val_of "$out" VK)" "$want4" "$sh_bin case4 mixed order & selective rewrite"
  got_nv=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t4/run/gearlever-cosmic-vk-icd/10_nvidia_icd.json")
  got_mini=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t4/run/gearlever-cosmic-vk-icd/20_mini_icd.json")
  assert_eq "$got_nv" "$work/lib64/libGLX_nvidia.so.0" "$sh_bin case4 nvidia library_path"
  assert_eq "$got_mini" "$work/lib64/libvulkan_mini.so" "$sh_bin case4 minified library_path"

  # 5. Same basename across XDG_CONFIG_HOME (higher priority) and XDG_DATA_DIRS + fallback to XDG_CACHE_HOME.
  t5="$work/t5-$sh_bin"
  mkdir -p "$t5/cfg/vulkan/icd.d" "$t5/share/vulkan/icd.d" "$t5/cache" "$t5/home"
  printf '{"ICD":{"library_path":"libvulkan_radeon.so"}}' > "$t5/cfg/vulkan/icd.d/radeon_icd.json"
  printf '{"ICD":{"library_path":"libvulkan_lvp.so"}}' > "$t5/share/vulkan/icd.d/radeon_icd.json"
  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t5/home" \
    XDG_CONFIG_HOME="$t5/cfg" XDG_DATA_DIRS="$t5/share" XDG_CACHE_HOME="$t5/cache" \
    LD_LIBRARY_PATH="$work/lib64")
  want5="$t5/cache/gearlever-cosmic-vk-icd/radeon_icd.json:$t5/cache/gearlever-cosmic-vk-icd/1-radeon_icd.json"
  assert_eq "$(val_of "$out" VK)" "$want5" "$sh_bin case5 duplicate basename & XDG_CACHE_HOME fallback"
  got_first=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t5/cache/gearlever-cosmic-vk-icd/radeon_icd.json")
  got_second=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t5/cache/gearlever-cosmic-vk-icd/1-radeon_icd.json")
  assert_eq "$got_first" "$work/lib64/libvulkan_radeon.so" "$sh_bin case5 config priority"
  assert_eq "$got_second" "$work/lib64/libvulkan_lvp.so" "$sh_bin case5 data second"

  # 6. Unresolvable bare SONAME -> clean no-op under set -e.
  t6="$work/t6-$sh_bin"
  mkdir -p "$t6/share/vulkan/icd.d" "$t6/run" "$t6/home"
  printf '{"ICD":{"library_path":"libvulkan_definitely_missing_999.so"}}' > "$t6/share/vulkan/icd.d/missing.json"
  out=$(run_hook "$sh_bin" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t6/home" \
    XDG_RUNTIME_DIR="$t6/run" XDG_DATA_DIRS="$t6/share" \
    LD_LIBRARY_PATH="$work/lib64")
  assert_eq "$(val_of "$out" VK)" "" "$sh_bin case6 unresolvable bare is no-op"
  [[ ! -e "$t6/run/gearlever-cosmic-vk-icd" ]] || fail "$sh_bin case6 created cache dir"
  pass=$((pass + 1))

  # 7. ldconfig -p fallback finds 64-bit lib outside standard dirs and ignores 32-bit entry.
  t7="$work/t7-$sh_bin"
  mkdir -p "$t7/share/vulkan/icd.d" "$t7/run" "$t7/home"
  printf '{"ICD":{"library_path":"libvulkan_ldcfg_only.so"}}' > "$t7/share/vulkan/icd.d/ldcfg.json"
  out=$(env -i PATH="$work/fakebin:$PATH" _GLC_VK_SYSCONFDIR="$work/empty-etc" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t7/home" \
    XDG_RUNTIME_DIR="$t7/run" XDG_DATA_DIRS="$t7/share" \
    "$sh_bin" -e -c '. "$HOOK"; printf "VK=%s\n" "${VK_DRIVER_FILES:-}"')
  assert_eq "$(val_of "$out" VK)" "$t7/run/gearlever-cosmic-vk-icd/ldcfg.json" "$sh_bin case7 ldconfig fallback"
  got_ldcfg=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ICD"]["library_path"])' \
    "$t7/run/gearlever-cosmic-vk-icd/ldcfg.json")
  assert_eq "$got_ldcfg" "$work/ldcfg64/libvulkan_ldcfg_only.so" "$sh_bin case7 ldconfig 64-bit picked"

  # 8. Direct `gearlever-cosmic-cli` arg skips hook.
  t8="$work/t8-$sh_bin"
  mkdir -p "$t8/run" "$t8/home"
  out=$(env -i PATH="$PATH" _GLC_VK_SYSCONFDIR="$work/empty-etc" \
    HOOK="$hook" APPDIR="$appdir" HOME="$t8/home" \
    XDG_RUNTIME_DIR="$t8/run" XDG_DATA_DIRS="$t1/share" LD_LIBRARY_PATH="$work/lib64" \
    "$sh_bin" -e -c 'set -- gearlever-cosmic-cli --version; . "$HOOK"; printf "VK=%s\n" "${VK_DRIVER_FILES:-}"')
  assert_eq "$(val_of "$out" VK)" "" "$sh_bin case8 cli arg skips hook"
done

echo "PASS ($pass checks across: ${shells[*]})"
