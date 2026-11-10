#!/bin/bash
# Usage: docker run ... secpar-hk [shell | session | CMD...]
source /opt/hk/session.sh || exit 1
case "${1:-shell}" in
  session) sleep infinity ;;
  shell) exec bash ;;
  *) exec "$@" ;;
esac
