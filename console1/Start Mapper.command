#!/bin/bash
cd "$(dirname "$0")"
clear
.venv/bin/python map_console1.py
echo
read -p "Press Enter to close."
