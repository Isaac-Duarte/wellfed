"""The entry's store by the beta upgrade rule: version 1 reads as its
sample, a newer minor is read with its additions kept, a newer major is
read as nothing and never written, and a store in another shape (a value
the bridge would refuse among them) is set aside under a numbered name, or
left untouched when that fails."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from homeassistant.core import HomeAssistant
import pytest
from pytest_homeassistant_custom_component.common import async_fire_time_changed

from custom_components.wellfed.const import DOMAIN
from custom_components.wellfed.kept import kept, set_aside
import wellfed_homeassistant as bridge

ENTRY = "an-entry"
KEY = f"{DOMAIN}.{ENTRY}"
SAMPLE: dict[str, Any] = json.loads(
    (Path(__file__).parent / "fixtures" / "kept-v1.json").read_text()
)


def _stored(version: int, minor: int, data: dict[str, Any]) -> dict[str, Any]:
    return {"version": version, "minor_version": minor, "key": KEY, "data": data}


async def _flushed(hass: HomeAssistant) -> None:
    """Run the store's delayed write, if one is due."""
    async_fire_time_changed(hass)
    await hass.async_block_till_done()


def _a_pending() -> bridge.Pending:
    return bridge.Pending(
        "01a0c450-6c00-7000-8000-000000000003", 3, 1790000002000, None, False, None
    )


async def test_version_1_reads_as_its_sample(
    hass: HomeAssistant, hass_storage: dict[str, Any]
) -> None:
    hass_storage[KEY] = json.loads(json.dumps(SAMPLE))
    store = kept(hass, ENTRY)
    await store.load()

    pending = [
        (p.id, p.portions, p.sent_ms, p.after, p.reported, p.released_ms) for p in store.pending
    ]
    assert pending == [
        ("01a0c450-6c00-7000-8000-000000000001", 2, 1790000000000, 41, True, 1790000900000),
        ("01a0c450-6c00-7000-8000-000000000002", 1, 1790000001000, None, False, None),
    ]
    assert store.answered == [41, 9]
    assert store.leftover_key == bytes([1] * 32)


async def test_a_newer_minor_reads_and_keeps_what_it_added(
    hass: HomeAssistant, hass_storage: dict[str, Any]
) -> None:
    data = {**SAMPLE["data"], "added": {"by": "a newer minor"}}
    hass_storage[KEY] = _stored(1, 2, data)
    store = kept(hass, ENTRY)
    await store.load()
    assert store.answered == [41, 9]

    store.add_pending(_a_pending())
    await _flushed(hass)

    saved = hass_storage[KEY]
    assert saved["minor_version"] == 2, "saved at the newer minor"
    assert saved["data"]["added"] == {"by": "a newer minor"}
    assert len(saved["data"]["pending"]) == 3


async def test_a_newer_major_is_read_as_nothing_and_never_written(
    hass: HomeAssistant, hass_storage: dict[str, Any], caplog: pytest.LogCaptureFixture
) -> None:
    newer = _stored(2, 1, {"of": "a newer shape"})
    hass_storage[KEY] = json.loads(json.dumps(newer))
    store = kept(hass, ENTRY)
    await store.load()
    assert store.pending == []
    assert "newer version of the integration" in caplog.text

    store.add_pending(_a_pending())
    store.set_answered([7])
    await _flushed(hass)

    assert hass_storage[KEY] == newer, "left as it was"


async def test_a_store_that_cannot_be_set_aside_is_never_saved_over(
    hass: HomeAssistant, hass_storage: dict[str, Any], caplog: pytest.LogCaptureFixture
) -> None:
    # The test's storage is in memory, so there is no file to copy aside.
    other = _stored(1, 1, {"pending": [{"id": "x"}]})
    hass_storage[KEY] = json.loads(json.dumps(other))
    store = kept(hass, ENTRY)
    await store.load()
    assert store.pending == []
    assert "could not be set aside" in caplog.text

    store.add_pending(_a_pending())
    await _flushed(hass)

    assert hass_storage[KEY] == other, "left as it was"


def _with_pending(**changes: Any) -> dict[str, Any]:
    """The sample's data, its first pending feed changed."""
    data: dict[str, Any] = json.loads(json.dumps(SAMPLE["data"]))
    data["pending"][0].update(changes)
    return data


@pytest.mark.parametrize(
    "data",
    [
        _with_pending(portions=300),
        _with_pending(portions=0),
        _with_pending(sent_ms=2**63),
        _with_pending(after=-1),
        _with_pending(released_ms=2**63),
        _with_pending(released_ms="soon"),
        {
            **SAMPLE["data"],
            "pending": [{k: v for k, v in SAMPLE["data"]["pending"][1].items() if k != "after"}],
        },
        {**SAMPLE["data"], "answered": [-1]},
        {**SAMPLE["data"], "answered": [2**32]},
        {**SAMPLE["data"], "leftover_key": "not a key"},
    ],
)
async def test_a_value_the_bridge_would_refuse_reads_as_another_shape(
    hass: HomeAssistant,
    hass_storage: dict[str, Any],
    caplog: pytest.LogCaptureFixture,
    data: dict[str, Any],
) -> None:
    hass_storage[KEY] = _stored(1, 1, data)
    store = kept(hass, ENTRY)
    await store.load()

    assert store.pending == [] and store.answered == [] and store.leftover_key is None
    assert "not in the shape it writes" in caplog.text


def test_set_aside_copies_to_the_next_free_name_and_never_replaces(tmp_path: Path) -> None:
    path = tmp_path / KEY
    path.write_bytes(b"first")
    assert set_aside(str(path)) == f"{path}.bad-1"

    path.write_bytes(b"second")
    assert set_aside(str(path)) == f"{path}.bad-2"

    assert Path(f"{path}.bad-1").read_bytes() == b"first"
    assert Path(f"{path}.bad-2").read_bytes() == b"second"
    assert path.read_bytes() == b"second", "left for the next save to replace"
