#!/bin/bash
cd "$(dirname "$0")"
pkill -f console1_soundcraft.py 2>/dev/null
.venv/bin/python monitor.py
read -p "Press Enter to close."
