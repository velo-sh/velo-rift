# Internal Branding & Hierarchy Specification

This document clarifies the relationship between the different layers of the project and the reasoning behind our naming conventions. This is an internal reference to ensure consistency across development and communication.

---

## 🚫 Velo Namespace Protection Rule

**CRITICAL REQUIREMENT**: Velo Rift™ is a derivative/sub-project of the parent **Velo** project. To avoid name collisions and user confusion, **no binary, command, or public identifier in Velo Rift™ may use the bare name `velo`**.

All public-facing names must be qualified or distinct (e.g., using the `vrift`™ prefix).

---

## 🏗 The Project Layers

| Layer | Name | Definition | Scope |
| :--- | :--- | :--- | :--- |
| **Product** | **Velo Rift™** | The top-level product brand. | CLI (`vrift`™), Daemon (`vriftd`), UX, and Integration. |
| **Capability** | **VeloVFS** | The core technical capability. | Virtual File System logic (Link Farm, Shim, OverlayFS). |
| **Engine** | **VeloVFS Core** | The lowest-level storage engine. | Also known as **TheSource**. Handles Content-Addressable Storage (CAS). |

---

## 🕶 The Storage: "TheSource"

Inspired by *The Matrix*, the physical storage location for all blobs (Content-Addressable Storage) is named **TheSource**.

### 1. Environment Variable: `VR_THE_SOURCE`
*   **Definition**: The root directory where Velo Rift™ stores all deduplicated blobs and metadata.
*   **Rule**: Replaces the old `VELO_CAS_ROOT` to maintain the "No Bare Velo" namespace rule.

### 2. Default Path: `/var/vrift/the_source`
*   **Definition**: The standard system location for TheSource.

---

## 🧬 Origins & Derivation

**VeloVFS** is derived from the original **Velo runtime**. 

*   **Velo Runtime**: A broader execution environment for high-performance computing (The parent project).
*   **Velo Rift™**: A specialized standalone product that extracts the VeloVFS core capabilities to provide a dedicated "Data Virtualization Layer" for any runtime.

---

## ⚡️ CLI Naming: `vrift`™ and `vriftd`

To distinguish Velo Rift™ from its parent (Velo) and provide a modern, brand-forward experience, we use the following naming convention:

### 1. The CLI: `vrift`™
*   **Definition**: Velo **Rift** CLI.
*   **Reasoning**: High brand recall. Directly shorthand for the product name "Velo Rift™". 

### 2. The Daemon: `vriftd`
*   **Definition**: Velo Rift™ Daemon.
*   **Reasoning**: Logical extension for background processes. Adding 'd' is the Unix standard for daemons. It is cleaner and more concise than `vrift-daemon`.

---

## 🎯 Positioning for the User

From the user's perspective, they are using **Velo Rift™**. 
- They install **Velo Rift™**.
- They run **`vrift`**™ commands.
- They get **VeloVFS** acceleration.
- Their data lives in **TheSource**.

The internal distinction between the Capability and the Core Engine is maintained to ensure modularity in the code (e.g., `velo-cas` crate vs `velo-runtime` crate) but is secondary to the "Velo Rift™" product experience.
