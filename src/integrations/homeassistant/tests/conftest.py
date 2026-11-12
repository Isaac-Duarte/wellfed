"""A simulated feeder, and the owner's server in front of it, for the
integration's tests.

The simulator is the workspace's `wellfed-feeder-sim`, which runs the real
feeder's pairing and feeding owner; invites come from the `wellfed` CLI, as
an owner makes them. The server is the workspace's `wellfed-server`, on
localhost with its own data directory and no mDNS. All three are built by
`make check-homeassistant`, or found under `WELLFED_BIN_DIR`.
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
from typing import Any
import urllib.request

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


# The server's first owner, as the tests sign in.
OWNER = "ada"
OWNER_PASSWORD = "a long test password"


@dataclass
class Server:
    url: str
    process: subprocess.Popen[bytes]
    # The owner's session cookie, from setup.
    cookie: str = ""

    def call(self, method: str, path: str, body: Any = None) -> Any:
        """One API call as the owner; its JSON answer, or `None` when empty."""
        request = urllib.request.Request(
            f"{self.url}/api/v1{path}",
            method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={
                "Origin": self.url,
                "Content-Type": "application/json",
                "Cookie": f"wellfed_session={self.cookie}",
            },
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            if not self.cookie:
                for header in response.headers.get_all("Set-Cookie") or []:
                    name, _, rest = header.partition("=")
                    if name == "wellfed_session":
                        self.cookie = rest.split(";", 1)[0]
            raw = response.read()
        return json.loads(raw) if raw else None

    def adopt(self, simulator: Simulator) -> dict[str, Any]:
        """Adopt `simulator` with an owner invite, and wait until the server reaches it."""
        feeder: dict[str, Any] = self.call(
            "POST",
            "/feeders",
            {"invite": simulator.invite("owner"), "name": "kitchen", "address": simulator.address},
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.call("GET", "/feeders")[0]["connection"] == "online":
                return feeder
            time.sleep(0.1)
        pytest.fail("the server did not reach the feeder")


@pytest.fixture
def server(tmp_path: Path, socket_enabled: None) -> Iterator[Server]:
    binary = _binary("wellfed-server")
    port = _free_port()
    process = subprocess.Popen(
        [
            binary,
            "--data",
            tmp_path / "server",
            "--listen",
            f"127.0.0.1:{port}",
            "--no-discovery",
            "--no-advertise",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    url = f"http://127.0.0.1:{port}"
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            urllib.request.urlopen(f"{url}/health", timeout=0.5).close()
            break
        except OSError:
            time.sleep(0.1)
    else:
        process.kill()
        pytest.fail("the server did not start listening")

    running = Server(url=url, process=process)
    running.call("POST", "/setup", {"name": OWNER, "password": OWNER_PASSWORD, "zone": "UTC"})
    yield running

    process.kill()
    process.wait(timeout=10)
