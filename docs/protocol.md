# RipGrok Tunnel Protocol (RGTP)

- [RipGrok Tunnel Protocol (RGTP)](#ripgrok-tunnel-protocol-rgtp)
  - [Introduction](#introduction)
  - [Overview](#overview)
    - [The client](#the-client)
    - [The server](#the-server)
  - [Connections](#connections)
    - [Initiating a connection](#initiating-a-connection)
    - [ControlConnection](#controlconnection)
      - [ClientControlHello](#clientcontrolhello)
      - [ServerControlHello](#servercontrolhello)
        - [ServerControlHello::Error](#servercontrolhelloerror)
        - [ServerControlHello::Listening](#servercontrolhellolistening)
      - [ServerControlCommand](#servercontrolcommand)
        - [ServerControlCommand::StartTunnelConnection](#servercontrolcommandstarttunnelconnection)
    - [TunnelConnection](#tunnelconnection)
  - [Types](#types)
    - [Tunnel Specifiers](#tunnel-specifiers)
      - [TunnelSpecifier](#tunnelspecifier)
      - [ResolvedTunnelSpecifier](#resolvedtunnelspecifier)
      - [TunnelType](#tunneltype)

## Introduction

The RipGrok Tunnel Protocol (RGTP) is a lightweight, multiplexed protocol designed to facilitate secure and flexible tunneling of raw and protocol-specific traffic from a local client through a remote server. The protocol allows a client behind NAT or firewall to expose services via a publicly accessible endpoint on the server.

## Overview

This document describes the RGTP protocol. The RGTP protocol defines all communication between the ripgrok client and ripgrok server.

### The client

The client is responsible for initiating connections and requesting tunnels. [More details forthcoming.]

### The server

The server accepts incoming connections and manages tunnel lifecycle. [More details forthcoming.]

RGTP defaults to using TLS over port 7267. However, it can also make use of TCP for testing or non-production scenarios.

RGTP over TLS uses the URI scheme `rgtps://`.
RGTP over plain TCP uses the URI scheme `rgtp://`.

## Connections

There are two types of connections between the ripgrok client and ripgrok server.

1. [**ControlConnection**](#controlconnection): This is the connection that is used to send commands to/from the server.
2. [**TunnelConnection**](#tunnelconnection): This is the connection that actually tunnels data between the ripgrok server and ripgrok client.

### Initiating a connection

As the ripgrok server only exposes a single port, the ripgrok client begins a connection by sending a **ClientHello**, which specifies which type of connection to initialize.

The **ClientHello** has a length of a single byte, and does not contain any data besides which type of connection to initiate.

| ClientHello Variant         | Value | Description                    |
| --------------------------- | ----- | ------------------------------ |
| **INIT_CONTROL_CONNECTION** | 0x41  | Initiate **ControlConnection** |
| **INIT_TUNNEL_CONNECTION**  | 0x42  | Initiate **TunnelConnection**  |

### ControlConnection

> All messages in a ControlConnection are UTF-8 encoded JSON objects, delimited by newline (`\n`). This format allows for streaming-friendly, line-delimited parsing.

A **ControlConnection** starts with a handshake, where the client sends a [**ClientControlHello**](#clientcontrolhello), to which the server responds with a [**ServerControlHello**](#servercontrolhello).

If the handshake is unsuccessful (server replies with [**ServerControlHello::Error**](#servercontrolhelloerror)), the server closes the connection.

If the handshake is successful (server replies with [**ServerControlHello::Listening**](#servercontrolhellolistening)), the rest of the connection is one-sided where the server sends [**ServerControlCommand**](#servercontrolcommand).

#### ClientControlHello

| Field      | Type                                  | Description               |
| ---------- | ------------------------------------- | ------------------------- |
| specifiers | [TunnelSpecifier](#tunnelspecifier)[] | List of tunnel specifiers |

Example:

To ask the server to open it's port 5678:

```json
{
    "specifiers": [
        {
            "tunnel_type": "tcp",
            "client_port": 1234,
            "server_port": 5678,
        }
    ]
}
```

#### ServerControlHello

| Field | Type | Description                                     |
| ----- | ---- | ----------------------------------------------- |
| type  | str  | Describes the variant of **ServerControlHello** |

Depending on the `type` field, more fields are required:

##### ServerControlHello::Error

| Field  | Type | Description      |
| ------ | ---- | ---------------- |
| type   | str  | Always `"Error"` |
| reason | str  | Error reason     |

Example:

```json
{
    "type": "Error",
    "reason": "Some reason that the handshake failed"
}
```

##### ServerControlHello::Listening

| Field       | Type                                                  | Description                                                      |
| ----------- | ----------------------------------------------------- | ---------------------------------------------------------------- |
| type        | str                                                   | Always `"Listening"`                                             |
| specifiers  | [ResolvedTunnelSpecifier](#resolvedtunnelspecifier)[] | List of tunnels                                                  |
| public_host | str \| null                                           | The public host that the tunnelled services will be reachable on |

Example:

```json
{
    "type": "Listening",
    "specifiers": [
        {
            "tunnel_type": "Tcp",
            "client_port": 1234,
            "server_port": 5678,
        }
    ]
}
```

#### ServerControlCommand

| Field | Type | Description                                       |
| ----- | ---- | ------------------------------------------------- |
| type  | str  | Describes the variant of **ServerControlCommand** |

Depending on the `type` field, more fields are required:

##### ServerControlCommand::StartTunnelConnection

| Field         | Type | Description                                    |
| ------------- | ---- | ---------------------------------------------- |
| type          | str  | Always `"StartTunnelConnection"`               |
| connection_id | u64  | A random connection id                         |
| server_port   | u16  | Which port the server received a connection on |

### TunnelConnection

A **TunnelConnection** starts with the client sending the raw `connection_id` (`u64`, 8 bytes, big endian). The `connection_id` should correspond to a `connection_id` that the client recieved in a [ServerControlCommand::StartTunnelConnection](#servercontrolcommandstarttunnelconnection).

After the `connection_id` has been sent, the rest of the connection is treated as a bidirectional binary stream between the client and server. There is no additional framing, message boundaries, or metadata — all bytes after the connection_id are passed through verbatim.

## Types

### Tunnel Specifiers

A tunnel specifier is a description of a tunnel that the client requests the server to initiate.

A [**TunnelSpecifier**](#tunnelspecifier) is sent by the client to the server to specify which port the client wants the server to listen on.

A [**ResolvedTunnelSpecifier**](#resolvedtunnelspecifier) is sent by the server to the client to confirm that a tunnel is active, as well as telling the client, which random port was chosen, if the client did not request a specific port.

#### TunnelSpecifier

| Field       | Type                      | Description                                                                                    |
| ----------- | ------------------------- | ---------------------------------------------------------------------------------------------- |
| tunnel_type | [TunnelType](#tunneltype) | Type of the tunnel                                                                             |
| client_port | u16                       | Which port the tunnel resolves to on the client                                                |
| server_port | u16 \| null               | Which port the server should listen on. If `null`, the server chooses a random available port. |

#### ResolvedTunnelSpecifier

| Field       | Type                      | Description                                     |
| ----------- | ------------------------- | ----------------------------------------------- |
| tunnel_type | [TunnelType](#tunneltype) | Type of the tunnel                              |
| client_port | u16                       | Which port the tunnel resolves to on the client |
| server_port | u16                       | Which port the server is listening on           |

#### TunnelType

| TunnelType Variant | Value   | Description                                                                                        |
| ------------------ | ------- | -------------------------------------------------------------------------------------------------- |
| TCP                | "Tcp"   | Server exposes raw TCP port, where data is forwarded to service                                    |
| HTTPS              | "Https" | Server exposes HTTPS server, where requests are proxied to http service. **Note: Not implemented** |
