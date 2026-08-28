# Architecture

The `user-profile` module is the single authoritative owner of user identity
display on the client side.  It communicates with the REST API and never
reads the database directly.  Downstream components (avatar widget, nav bar,
settings page) consume `ProfileViewModel`; they never touch `UserProfile`.

# Boundaries

- Never call `/api/admin/*` endpoints from this module.
- Never store auth tokens, those live in `auth-context`.
- All API calls must go through `fetchUserProfile` or `updateProfile`; no raw
  `fetch` calls to `/api/users/...` from other modules.

# Stack

TypeScript, browser-native `fetch`, no framework dependency in this module.

# Owner

team: frontend-core
contact: #frontend-platform
