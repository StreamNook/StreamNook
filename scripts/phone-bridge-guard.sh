#!/usr/bin/env bash
rc=0; rg -q --fixed-strings 'PHONE_LINK_BRIDGE' "$1" || rc=$?; [ "$rc" -eq 1 ] || { echo "error: Slice B's phone link bridge is still in the tree; build from a tree with Slice C" >&2; exit 1; }
