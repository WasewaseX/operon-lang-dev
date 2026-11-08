"""Unit pins for oracle-internal contracts the 3.5k-program differential
corpus underexercises (added with the master-ci gates: the python job seeds
pytest collection). These are REGRESSION pins on the current, Rust-verified
behavior — the behavioral gates of record remain the sharded differential +
vm_parity suites, which prove three-engine byte-identity end to end.

Pinned here:
- v_repr: the canonical representation law (scalar forms, list/map shapes)
- deep_eq: structural equality laws, including the two laws the
      corpus historically undercovered — bool/int distinctness and the
      >2^53 Int/Float exactness rule (no f64 rounding on the oracle).
"""

import importlib.util
import os

import pytest

_SPEC = importlib.util.spec_from_file_location(
    "oracle_under_test",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bootstrap", "oracle.py"),
)
assert _SPEC is not None and _SPEC.loader is not None, "bootstrap/oracle.py not found"
oracle = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(oracle)


# ---------------------------------------------------------------- v_repr
@pytest.mark.parametrize(
    "value, expected",
    [
        (3, "3"),
        (-7, "-7"),
        (2.5, "2.5"),
        ("a", '"a"'),
        (True, "true"),
        (False, "false"),
        (None, "null"),
        ([1, "x"], '[1, "x"]'),
        ({"a": 1}, "{a: 1}"),
        ([1, [2, "y"]], '[1, [2, "y"]]'),
    ],
)
def test_v_repr_canonical_forms(value, expected):
    assert oracle.v_repr(value) == expected


def test_v_repr_map_is_insertion_ordered():
    # the Z-MAPORDER law: mixed scalar+container maps render in pure
    # insertion order (side entries included)
    m = {"b": 1, "a": [2]}
    assert oracle.v_repr(m) == "{b: 1, a: [2]}"


# --------------------------------------------------------------- deep_eq
@pytest.mark.parametrize(
    "a, b, expected",
    [
        ([1, 2], [1, 2], True),
        ([1, [2, "x"]], [1, [2, "x"]], True),
        ({"a": 1, "b": [2]}, {"b": [2], "a": 1}, True),  # order-independent
        ([1, 2], [2, 1], False),
        ({"a": 1}, {"a": 2}, False),
        ("a", "a", True),
        (1, True, False),  # bool/int are distinct classes in the oracle
        (True, True, True),
        (2**53 + 1, float(2**53 + 1), False),  # >2^53: exact, never rounded
        (2.0, 2, True),  # exact-representable float == int
    ],
)
def test_deep_eq_laws(a, b, expected):
    assert oracle.deep_eq(a, b) is expected
    assert oracle.deep_eq(b, a) is expected  # symmetry


def test_deep_eq_nested_container_key_roundtrip():
    # Z-104/Z-214 family: container keys survive insert/lookup round-trips
    m = {}
    key = [1, "k"]
    m[repr(key)] = "val"  # the oracle two-store repr path
    assert oracle.deep_eq([1, "k"], [1, "k"]) is True
