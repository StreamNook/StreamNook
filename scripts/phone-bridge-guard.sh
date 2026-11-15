#!/usr/bin/env bash
rc=0; rg -q --fixed-strings 'PHONE_LINK_BRIDGE' "$1" || rc=$?; [ "$rc" -eq 1 ] || { echo "error: the phone link bridge is still in the tree; Android builds wait for the phone's own link doors" >&2; exit 1; }
