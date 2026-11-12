"""Constants for the Wellfed integration."""

from typing import Final

import wellfed_homeassistant as bridge

DOMAIN: Final = "wellfed"

# The entry's version, its major: a newer minor of it is read as it is.
ENTRY_VERSION: Final = 2
ENTRY_MINOR_VERSION: Final = 1

# Version 2 of the entry's data (version 1 had no routes, and is not
# migrated): `route` and `identity` on both routes,
# then `secret` and `address` directly, or `url`, `token`, `token_id` and
# `feeder_id` through the owner's server.
CONF_ROUTE: Final = "route"
CONF_IDENTITY: Final = "identity"
CONF_SECRET: Final = "secret"
CONF_ADDRESS: Final = "address"
CONF_URL: Final = "url"
CONF_TOKEN: Final = "token"
CONF_TOKEN_ID: Final = "token_id"
CONF_FEEDER_ID: Final = "feeder_id"

# Form fields that are never stored.
CONF_INVITE: Final = "invite"
CONF_FEEDER: Final = "feeder"

# How the entry reaches its feeder: directly over the device API, or
# through the owner's server.
ROUTE_DIRECT: Final = "direct"
ROUTE_SERVER: Final = "server"

# The label the feeder lists this client under, and the server this
# integration's token.
CLIENT_LABEL: Final = "home-assistant"

# Fired on the bus for every event the feeder records as it happens.
EVENT_FEEDER: Final = "wellfed_event"

# Repairs that outlive an entry or a route: a client key a move to the
# server route could not remove, and a token a deleted entry could not
# revoke.
LEFTOVER_KEY_ISSUE: Final = "leftover_key"
TOKEN_LEFT_ISSUE: Final = "token_left"

SERVICE_FEED: Final = "feed"
SERVICE_CANCEL_FEED: Final = "cancel_feed"
ATTR_PORTIONS: Final = "portions"
ATTR_FEED: Final = "feed"

# The feeder's own bounds on a feed's portions, from the bridge. services.yaml
# cannot import them, so a test pins its copy to these.
PORTIONS_MIN: Final = bridge.PORTIONS_MIN
PORTIONS_MAX: Final = bridge.PORTIONS_MAX
