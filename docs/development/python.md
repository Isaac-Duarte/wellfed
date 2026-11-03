# Python standard

These are the rules for the Home Assistant integration in `src/integrations/homeassistant/`, the only Python in the repository, why each exists, and what enforces it. [AGENTS.md](../../AGENTS.md#python-conventions) keeps the short form, and [the integration's README](../../src/integrations/homeassistant/README.md) explains how it is built and released.

Every rule marked with a tool runs in `make check-homeassistant`. It tests against Home Assistant 2026.10.4 on Python 3.14, pinned through `pytest-homeassistant-custom-component` in `tests/requirements.txt`, and recreates its venv on any other Python.

## Layout

```text
src/integrations/homeassistant/
  bridge/                        the Rust crate wellfed-homeassistant, built as the wellfed_homeassistant module
    wellfed_homeassistant.pyi    its stub, generated from the module and committed
  custom_components/wellfed/     the integration: a platform per file, the session pump, the flows
  tests/                         pytest under Home Assistant's harness, against the simulator
    fake.py                      a scripted bridge, for paths the simulator can't reach from Python
```

## Rules

| Rule | Why | Enforced by |
| --- | --- | --- |
| **No wire logic in Python.** Sessions, invites, request ids, retries, event cursors, release checks and installs live in the Rust bridge. Python moves updates into Home Assistant and hands writes to the bridge. | The feeder's rules exist once, shared with the app, the CLI and the server. A feed is sent once, from Rust. | Review; the bridge's tests |
| **Typed values only.** What the bridge returns is a class from its stub, never JSON. Compare its enums with `==`, since PyO3 may build a new object for each access. | A key read by hand is a quiet copy of a Rust type, and drifts. | mypy against the stub |
| **The stub is generated.** Regenerate `bridge/wellfed_homeassistant.pyi` after any change to the bridge's Python surface, and commit it with that change. Every wheel ships it. | Python is checked against what the module really exposes. | `check-homeassistant` fails on a stale stub |
| **Strict types.** `mypy --strict` over the integration and its tests, with `warn_unreachable`. | A wrong field name fails before a feeder is involved. | mypy |
| **One format, one lint set.** Ruff with pycodestyle, pyflakes, isort, bugbear, pyupgrade, simplify and Ruff's own rules, at 100 columns. | No formatting in review. | `ruff check`, `ruff format --check` |
| **Words come from translations.** Entity names, states and every error raised to the user carry a translation key. `strings.json` and `translations/en.json` stay identical. | Home Assistant shows them in the user's language, and core requires it. | Review; the quality scale |
| **A state is the feeder's.** An entity shows what the session last read, never the command it just sent. | A command that didn't land must not look as if it did. | Review; the entity tests |
| **Python only here.** No Python elsewhere under `src/`. | The host tools are Rust. | `cargo xtask check-repository` |

## Tests

| Layer | Covers | Where |
| --- | --- | --- |
| Bridge | The session, writes, settlement and installs, in Rust against the simulator | `bridge/tests/` |
| Flows | Pairing, zeroconf, reauth, reconfigure and the options | `tests/test_config_flow.py` |
| Entities | Each entity and action end to end against the simulator | `tests/test_entities.py`, `tests/test_integration.py` |
| Scripted | Repairs, a revoked key or a silent feeder at setup, installs, diagnostics, stale devices | `tests/test_fake_paths.py` |

The simulator tests start `wellfed-feeder-sim` and use the `wellfed` CLI as its owner, both from `WELLFED_BIN_DIR`, which `check-homeassistant` sets. Coverage of the integration must stay at 95% or above.

## Before a release

The host checks don't prove the integration works on a feeder. On a real feeder, with Home Assistant run from a scratch config:

1. Pair with a member invite, then with an owner invite.
2. Feed one portion from the Feed button, and check the bowl against the Feed event.
3. Turn the child lock and a meal plan off and on, and check the feeder agrees.
4. Revoke Home Assistant's key, and pair it again through the repair.

Then build the wheels with `make homeassistant-wheels`, install the musllinux one in a Home Assistant container of the target release, and check it imports.
