#!/usr/bin/env python3
"""Compatibility entrypoint: regenerate the current Inklura Desktop brand."""
from pathlib import Path
import runpy
runpy.run_path(str(Path(__file__).with_name("export-inklura-brand.py")), run_name="__main__")
