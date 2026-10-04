#!/bin/sh
# Apsis: read-only checks after a full-system restore (PLAN 6b.12, "Harness on apsis-test").
# Run as root on the restored machine: sudo tools/restore-check.sh
# Prints what it finds; changes nothing. Each "FAIL" line is something to look at.
set -u

ok() { echo "ok    $*"; }
fail() { echo "FAIL  $*"; }

state=/var/lib/apsis/restore
root_uuid=$(findmnt -no UUID / 2>/dev/null || true)
esp=/boot/efi/EFI/Pop_OS-$root_uuid
version=$(uname -r)

echo "== apsis restore check: $(date -Is), kernel $version, root $root_uuid"

# 1. The arm is gone: no link, no unit, no wants link, no drop-in, no helper copy.
for p in /system-update /etc/system-update /etc/systemd/system/apsis-restore.service \
         /etc/systemd/system/system-update.target.wants/apsis-restore.service \
         /etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf \
         "$state/apsis-helper" "$state/state.json"; do
  if [ -e "$p" ] || [ -L "$p" ]; then fail "still there: $p"; else ok "gone: $p"; fi
done
if [ -d /etc/systemd/system/pop-upgrade-init.service.d ]; then
  fail "drop-in folder left: /etc/systemd/system/pop-upgrade-init.service.d"
else
  ok "no drop-in folder"
fi
wants=/etc/systemd/system/system-update.target.wants
if [ -d "$wants" ] && [ -z "$(ls -A "$wants" 2>/dev/null)" ]; then fail "empty wants folder left: $wants"; else ok "no empty wants folder"; fi
if systemctl list-units --all 'apsis-disarm*' --no-legend 2>/dev/null | grep -q .; then
  fail "a disarm timer or service is still loaded"
else
  ok "no disarm timer"
fi

# 2. The result.
if [ -f "$state/result.json" ]; then
  ok "result.json:"; sed 's/^/        /' "$state/result.json"
else
  fail "no $state/result.json"
fi
if [ -d "$state/esp-backup" ]; then fail "esp-backup/ left in $state (a give-up keeps it on purpose)"; else ok "no esp-backup/ left"; fi

# 3. The ESP boots the kernel /boot links to, byte for byte, and its modules are there.
for pair in "vmlinuz vmlinuz.efi" "initrd.img initrd.img"; do
  set -- $pair
  link=/boot/$1; on_esp=$esp/$2
  target=$(readlink "$link" 2>/dev/null || true)
  if [ -z "$target" ]; then fail "$link isn't a link"; continue; fi
  file=/boot/$(basename "$target")
  if [ -f "$on_esp" ] && cmp -s "$file" "$on_esp"; then
    ok "$on_esp is $file"
  else
    fail "$on_esp differs from $file (or is missing)"
  fi
done
booted=$(readlink /boot/vmlinuz 2>/dev/null | sed 's/^vmlinuz-//')
if [ -n "$booted" ] && [ -d "/usr/lib/modules/$booted" ]; then ok "modules for $booted"; else fail "no modules for $booted"; fi
if [ -d "/usr/lib/modules/$version" ]; then ok "modules for the running kernel $version"; else fail "no modules for the running kernel $version"; fi
if [ "$booted" = "$version" ]; then ok "running the kernel the ESP boots"; else echo "note  running $version, the ESP boots $booted (a kernel rollback restarts into the new one; see result.json)"; fi
if [ -f /boot/efi/loader/entries/Pop_OS-current.conf ]; then ok "Pop_OS-current.conf"; else fail "no Pop_OS-current.conf"; fi
if [ -f /boot/efi/loader/entries/Pop_OS-oldkern.conf ]; then ok "Pop_OS-oldkern.conf (untouched by a restore)"; else echo "note  no Pop_OS-oldkern.conf (one kernel installed)"; fi

# 4. dpkg, the units, Pop's upgrade state.
if dpkg --audit >/dev/null 2>&1 && [ -z "$(dpkg --audit 2>/dev/null)" ]; then ok "dpkg --audit clean"; else fail "dpkg --audit:"; dpkg --audit 2>&1 | sed 's/^/        /'; fi
failed=$(systemctl --failed --no-legend 2>/dev/null | wc -l)
if [ "$failed" = 0 ]; then ok "0 failed units"; else fail "$failed failed units:"; systemctl --failed --no-legend | sed 's/^/        /'; fi
for u in acpid pop-upgrade; do
  echo "note  $u is-enabled: $(systemctl is-enabled "$u" 2>&1)"
done
if [ -e /upgrade-attempted ] || [ -e /pop-upgrade ]; then fail "Pop's upgrade left /upgrade-attempted or /pop-upgrade"; else ok "no Pop upgrade leftovers"; fi

# 5. The restore's own boot, if the previous boot was it.
echo "== journal of the previous boot, apsis-restore.service:"
journalctl -b -1 --no-pager -u apsis-restore.service 2>/dev/null | sed 's/^/        /' || echo "        (no previous boot in the journal)"
echo "== pop-upgrade-init in that boot (expect: skipped on ConditionPathExists=!/system-update/apsis-helper):"
journalctl -b -1 --no-pager -u pop-upgrade-init.service 2>/dev/null | sed 's/^/        /'
echo "== done"
