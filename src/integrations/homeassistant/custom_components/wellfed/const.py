"""Constants for the Wellfed integration."""

from typing import Final

import wellfed_homeassistant as bridge

DOMAIN: Final = "wellfed"

# The entry's version, its major: a newer minor of it is read as it is.
ENTRY_VERSION: Final = 2
ENTRY_MINOR_VERSION: Final = 1

# Version 2 of the entry's data (version 1, from test builds, is not
# migrated): the feeder's `identity`, the client key's `secret`, and the
# `address`, or none to find the feeder over mDNS.
CONF_IDENTITY: Final = "identity"
CONF_SECRET: Final = "secret"
CONF_ADDRESS: Final = "address"

# A form field that is never stored.
CONF_INVITE: Final = "invite"

# The label the feeder lists this client under.
CLIENT_LABEL: Final = "home-assistant"

# Fired on the bus for every event the feeder records as it happens.
EVENT_FEEDER: Final = "wellfed_event"

# A key a setup flow's join may have enrolled when its answer was lost, left
# when the flow ended: its repair outlives the flow, since nothing checks
# the key after.
LEFTOVER_KEY_ISSUE: Final = "leftover_key"
LEFTOVER_KEY_UNJOINED: Final = "leftover_key_unjoined"

SERVICE_FEED: Final = "feed"
SERVICE_CANCEL_FEED: Final = "cancel_feed"
ATTR_PORTIONS: Final = "portions"
ATTR_FEED: Final = "feed"

# The feeder's own bounds on a feed's portions, from the bridge. services.yaml
# cannot import them, so a test pins its copy to these.
PORTIONS_MIN: Final = bridge.PORTIONS_MIN
PORTIONS_MAX: Final = bridge.PORTIONS_MAX
