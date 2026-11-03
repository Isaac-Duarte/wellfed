"""Constants for the Wellfed integration."""

from typing import Final

DOMAIN: Final = "wellfed"

CONF_SECRET: Final = "secret"
CONF_IDENTITY: Final = "identity"
CONF_INVITE: Final = "invite"
CONF_ADDRESS: Final = "address"
CONF_ROLE: Final = "role"
# Owner mode's option: the keys the project signs releases with, hex.
CONF_PROJECT_KEYS: Final = "project_keys"

# The label the feeder lists this client under.
CLIENT_LABEL: Final = "home-assistant"

# Fired on the bus for every event the feeder records as it happens.
EVENT_FEEDER: Final = "wellfed_event"

SERVICE_FEED: Final = "feed"
SERVICE_CANCEL_FEED: Final = "cancel_feed"
SERVICE_ADD_RELEASE: Final = "add_release"
ATTR_PORTIONS: Final = "portions"
ATTR_FEED: Final = "feed"
ATTR_PATH: Final = "path"

# The feeder's own bounds on a feed's portions (wellfed_model::PORTIONS_MAX).
PORTIONS_MIN: Final = 1
PORTIONS_MAX: Final = 48
