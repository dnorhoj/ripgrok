# Protocol specification for ripgrok (RGP)

## Overview

TODO

## Connections

There are two types of connections between the client and server.

1. **ControlConnection**: This is the connection that is used to send commands to/from the server.
2. **TunnelConnection**: When the server requests the client to open a tunnel (through the **ControlConnection**), the client opens a **TunnelConnection** to the server. This connection is then 
