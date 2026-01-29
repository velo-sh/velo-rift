# The Velo World Model — One‑Page Whitepaper

## Software Needs a New World Model

In 2026, software is no longer something merely *run by humans*.

It is:
- Continuously built by CI systems
- Automatically executed by AI agents
- Dynamically scheduled across remote execution environments
- Strictly audited by security and compliance systems

Yet all of this still runs on a world model designed in the 1970s:

> **inode‑based file systems + mutable paths + implicit runtime state**

That model is no longer sufficient.

---

## What the Old World Gets Wrong

### 1. Paths Do Not Represent Truth

The same path:
```
/usr/lib/libssl.so
```
may refer to *entirely different binaries* on different machines.

As a result:
- Builds are not reproducible
- Tests are environment‑dependent
- Runtime behavior cannot be proven

---

### 2. Mutable State Is Not Verifiable

In today's systems, it is effectively impossible to answer:

> *"In exactly what world was this binary produced?"*

This becomes catastrophic in the age of AI agents, where:
- Execution must be auditable
- Behavior must be replayable
- Decisions must be explainable

---

### 3. The File System Is No Longer the Source of Truth

Reality is fragmented across:
- Lockfiles
- Caches
- Container layers
- Environment variables

The file system is merely the *final side effect*, not the authority.

---

## Velo's Core Insight

> **The problem is not that file systems are too slow.  
> The problem is that the world model itself is wrong.**

Velo does not attempt to patch the inode world.

Instead, it asks a more fundamental question:

> **What world should modern software actually live in?**

---

## The Velo World Model (First Principles)

### 1. Path = Content

Every path deterministically resolves to a **content hash**.

- Paths are no longer names
- Paths are **facts**

If two paths are equal, their contents are provably identical.

---

### 2. Snapshots Are First‑Class Citizens

The world is not a mutable "current state".

It is an **immutable snapshot**.

All activities:
- build
- test
- run
- agent execution

happen *inside* a snapshot.

---

### 3. Execution = Program + World

Programs no longer "depend on environments".

They execute inside a **provable world state**.

This makes execution:
- reproducible
- auditable
- replayable

by construction.

---

## VeloVFS: Not a File System, but a World Projection

VeloVFS does not exist to store files.

It exists to:

> **Project an immutable software world into a mmap‑able, executable interface.**

There is:
- no inode traversal
- no runtime path ambiguity
- no "works on my machine" failure mode

---

## Why This Is Infrastructure for the Agent Era

For AI agents, the world must be:
- freezable (snapshot)
- forkable
- replayable
- auditable

Velo does not offer a faster file system.

It offers:

> **A world state that autonomous agents can trust.**

---

## What This Means for Human Developers

- Builds become deterministic by default
- Tests are naturally reproducible
- Supply‑chain security emerges automatically
- Debugging gains time‑travel semantics

You are no longer writing code that merely runs on *some machine*.

You are defining:

> **Software facts that hold inside a specific, provable world.**

---

## Velo's Ambition — and Its Boundaries

Velo is **deliberately narrow**.

### What Velo Does NOT Attempt:

| Domain | Owner | Velo's Stance |
|--------|-------|---------------|
| Build graph semantics | Bazel, Buck | We don't model dependencies between targets |
| Functional evaluation | Nix | We don't evaluate derivations |
| Process isolation | Docker, gVisor | We don't sandbox syscalls |
| Distributed consensus | etcd, Raft | We don't coordinate clusters |
| Package resolution | uv, npm, cargo | We don't solve version conflicts |

### What Velo DOES Do:

> **Make file access instant, reproducible, and verifiable.**

That's it. One thing. Done well.

---

## One‑Sentence Summary

> **If software is becoming an autonomous, living entity,  
> it requires a world that does not shift beneath its feet.**

**Velo is that world.**

---

*Document Version: 2.0*  
*Last Updated: 2026-01-29*
