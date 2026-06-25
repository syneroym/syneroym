# D-02-04: Docker Image Scope and Base Image

**Status**: Accepted

**Context**: 
Requirement `[FND-DEP]` specifies official Docker images pre-configured for the community. The codebase currently lacks Docker infrastructure. We needed to decide which binaries to containerize, the base image, and the target architectures.

**Decision**: 
We will containerize both `syneroym-substrate` and `roymctl` in a single official Docker image. We will use `debian-slim` as the base image to balance minimal size with standard glibc compatibility and basic debugging utilities. The images will be built for both `linux/amd64` and `linux/arm64` architectures.

**Consequences**: 
- **Enables**: Easy deployment for community operators on standard cloud VMs and ARM-based devices (e.g., Raspberry Pi 4). Includes `roymctl` for convenient local administration via `docker exec`.
- **Defers**: Distroless or scratch containers for maximum security hardening, which can be explored in later milestones.

**Implementation Notes**: 
- Add a `Dockerfile` at the workspace root.
- Add a GitHub Actions workflow (`docker.yml`) to build and push multi-arch manifests using `buildx`.
