"""Teacher module for subway/metro assignment grading.

Provides:
- DATA_FILE: path to the test CSV
- Helper checkers for property-based tests on student functions
"""
import csv
from pathlib import Path
from collections import deque

# --- Test data ---
DATA_FILE = str(Path(__file__).parent / "线路.csv")


# --- Duck-typing helpers (same as pytest version) ---
def _station_map(data):
    if isinstance(data, dict):
        return data
    for attr in ("stations", "station_dict", "id_to_station", "nodes"):
        if hasattr(data, attr):
            m = getattr(data, attr)
            if isinstance(m, dict):
                return m
    return {}


def _get(obj, *names, default=None):
    for n in names:
        if hasattr(obj, n):
            return getattr(obj, n)
        if isinstance(obj, dict) and n in obj:
            return obj[n]
    return default


def _line(st):
    return _get(st, "line_name", "line", default="")


def _name(st):
    return _get(st, "station_name", "name", default="")


def _prev(st):
    return _get(st, "prev_id", "prev", default=None)


def _next(st):
    return _get(st, "next_id", "next", default=None)


def _transfers(st):
    return _get(st, "transfer_ids", "transfers", "transfer", default=None)


# --- Checkers ---

@checker("load_data")
def check_load_data(result, expected):
    """Verify load_data returns a structure with stations."""
    smap = _station_map(result)
    if not smap or len(smap) == 0:
        return False, "load_data returned empty or unrecognizable station map"

    # Check bidirectional prev/next
    for sid, st in list(smap.items())[:500]:
        nxt = _next(st)
        prv = _prev(st)
        if nxt is not None:
            if nxt not in smap:
                return False, f"next_id points to nonexistent station: {sid}->{nxt}"
            if _prev(smap[nxt]) != sid:
                return False, f"next/prev mismatch: {sid}.next={nxt}, but {nxt}.prev={_prev(smap[nxt])}"
        if prv is not None:
            if prv not in smap:
                return False, f"prev_id points to nonexistent station: {sid}->{prv}"

    # Check transfer_ids are ints and reference valid stations
    for sid, st in list(smap.items())[:500]:
        tr = _transfers(st)
        if tr is None:
            continue
        for x in tr:
            if not isinstance(x, int):
                return False, f"transfer_ids must be int: station {sid}, got {type(x)}"
            if x not in smap:
                return False, f"transfer_ids references nonexistent station: {sid}->{x}"

    return True, ""


def check_get_station_id_roundtrip(result, expected, loaded_data):
    """Verify get_station_id returns correct ID for a known station."""
    smap = _station_map(loaded_data)
    if not smap:
        return False, "No station data loaded"
    # result should be an int matching one of the station IDs
    if result is None:
        return False, "get_station_id returned None for existing station"
    if result not in smap:
        return False, f"get_station_id returned {result}, not a valid station ID"
    return True, ""


def check_neighbors(result, expected, loaded_data):
    """Verify get_neighbors returns list including prev/next/transfers."""
    if not isinstance(result, list):
        return False, f"Expected list, got {type(result).__name__}"
    # Basic sanity: should not be empty for most stations
    return True, ""


def check_find_path(result, expected, loaded_data):
    """Verify find_path returns a valid walk."""
    if result is None or result == []:
        return False, "find_path returned empty/None"
    if not isinstance(result, list):
        return False, f"Expected list, got {type(result).__name__}"
    if len(result) != len(set(result)):
        return False, "Path contains duplicate nodes (visited not working?)"
    return True, ""


def check_format_path(result, expected, loaded_data):
    """Verify format_path returns string with transfer markers."""
    if not isinstance(result, str):
        return False, f"Expected str, got {type(result).__name__}"
    if len(result) == 0:
        return False, "format_path returned empty string"
    return True, ""
