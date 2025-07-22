# One remote to rule them all: A native remote for Jujutsu

Author: [Isaac Corbrey](mailto:icorbrey@gmail.com)

## Summary

This design document proposes the creation of a first-party "Jujutsu server"
that the CLI can natively interop with as a remote. This will not replace
existing functionality that is in place for interacting with Git (or any other
DVCS we may support in the future), but instead facilitates interops with
centralized version control systems (CVCS) like Perforce, Subversion, and Team
Foundation Services.

## Goals and non-goals

### Goals

- Allow pushing/fetching from Jujutsu clients.
- Authenticate via some TBD method.
- Expose a `Backend`-like interface layer to be able to add support for various
  third-party version control systems in the future.
- Create an easily deployed and configured package for devops teams to set up.

### Non-goals

- We probably only need to store data that a Git remote would normally store
  when being used with Jujutsu (i.e. no op logs).

## Overview

**TK**

### Detailed Design

**TK**

## Alternatives considered

- We could implement support for CVCSs directly in the client itself.
  - Because most CVCSs don't have a concept of tree objects (and thus require
    on-the-fly conversion) doing this would make users have to wait a while when
    pushing or fetching for the conversion to complete.
  - This has the additional
    disadvantage of needing to be done on every client every time it interacts
    with the source system rather than being done once and stored in a format
    compatible with Jujutsu.
  - This would however probably ease the pain of implementing authentication
    between the Jujutsu remote and the target CVCS (which preferably should be
    able to be done per-user rather than as an integration or app).

## Related Work

This has technically already been implemented as a specialized server for
Google's Piper VCS. This is closed source and as such we can't use it as a base,
but we do have [Martin](mailto:martinvonz@google.com) to lean on for guidance.
