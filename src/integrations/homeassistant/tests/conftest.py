"""A simulated feeder for the integration's tests.

The simulator is the workspace's `wellfed-feeder-sim`, which runs the real
feeder's pairing and feeding owner; invites come from the `wellfed` CLI, as
an owner makes them. Both are built by `make check-homeassistant`, or found
under `WELLFED_BIN_DIR`.
"""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass
import json
import os
from pathlib import Path
import socket
import subprocess
import time

import pytest

REPOSITORY = Path(__file__).resolve().parents[4]
BIN_DIR = Path(os.environ.get("WELLFED_BIN_DIR", REPOSITORY / "target" / "debug"))


@pytest.fixture(autouse=True)
def auto_enable_custom_integrations(enable_custom_integrations: None) -> None:
    """Load `custom_components/wellfed`."""
    return


def _binary(name: str) -> Path:
    path = BIN_DIR / name
    if not path.exists():
        pytest.skip(f"{path} is not built: run `make check-homeassistant`")
    return path


def _free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port: int = probe.getsockname()[1]
        return port


@dataclass
class Simulator:
    address: str
    keys: Path
    cli: Path
    process: subprocess.Popen[bytes]

    def owner(self, *args: str) -> str:
        """A `wellfed` command as the simulator's owner; its output."""
        command = [str(self.cli), *args, "--keys", str(self.keys), "--address", self.address]
        done = subprocess.run(command, capture_output=True, text=True, check=True, timeout=60)
        return done.stdout

    def console(self, line: str) -> None:
        """A line typed at the simulator's console: a press, the adapter, the hopper."""
        assert self.process.stdin is not None
        self.process.stdin.write(f"{line}\n".encode())
        self.process.stdin.flush()

    def invite(self, role: str = "member") -> str:
        """A single-use invite from the simulator's owner."""
        return self.owner("invite", "--role", role).strip()


def _names_a_feeder(keys: Path) -> bool:
    """Whether the key file at `keys` records a feeder; a file being written
    reads as not yet."""
    try:
        feeders = json.loads(keys.read_text()).get("feeders")
    except (OSError, ValueError):
        return False
    return bool(feeders)


@pytest.fixture
def simulator(tmp_path: Path, socket_enabled: None) -> Iterator[Simulator]:
    cli = _binary("wellfed")
    sim = _binary("wellfed-feeder-sim")
    keys = tmp_path / "owner.json"
    # The CLI makes a key only when asked: this makes it, then fails to connect.
    subprocess.run(
        [cli, "recover", "--create-key", "--keys", keys, "--address", f"127.0.0.1:{_free_port()}"],
        capture_output=True,
        timeout=30,
    )
    assert keys.exists()

    port = _free_port()
    process = subprocess.Popen(
        [sim, "--state", tmp_path / "sim", "--listen", f"127.0.0.1:{port}", "--enroll-owner", keys],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    # Ready once the key file names the feeder: the simulator binds its port
    # before it starts and records its identity there, so a connection can
    # succeed while the owner's `wellfed` still finds no feeder to talk to.
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if _names_a_feeder(keys):
            try:
                socket.create_connection(("127.0.0.1", port), timeout=0.5).close()
                break
            except OSError:
                pass
        time.sleep(0.1)
    else:
        process.kill()
        pytest.fail("the simulator did not start, or did not record its identity")

    yield Simulator(address=f"127.0.0.1:{port}", keys=keys, cli=cli, process=process)

    process.kill()
    process.wait(timeout=10)
