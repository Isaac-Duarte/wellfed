"""Constants for the Wellfed integration."""

from typing import Final

DOMAIN: Final = "wellfed"

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

# The feeder's own bounds on a feed's portions (wellfed_model::PORTIONS_MAX).
PORTIONS_MIN: Final = 1
PORTIONS_MAX: Final = 48
