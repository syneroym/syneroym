# Architecture & Engineering Specification: Federated Data-Aware Authorization Engine (FDAE)

> ⚠️ **Status: Exploratory, Ideation**
>
> This approach to RBAC is more of an idea right now, and we might not do a full-fledged implementation of the same in this project. Most likely some bits and pieces

## 1. Introduction & Philosophy

The Federated Data-Aware Authorization Engine (FDAE) is a decentralized, high-performance authorization architecture engineered for WebAssembly (Wasm) host platforms.

Modern authorization engines force developers into a critical architectural trade-off:

- **Policy-Based Access Control (PBAC / Attribute-Based) engines** such as AWS Cedar and Open Policy Agent treat authorization strictly as a logical problem. They operate statelessly, forcing application code to handle the heavy lift of querying, assembling, and serializing complex relational data maps into JSON bundles before every API request.
- **Relationship-Based Access Control (ReBAC) engines** such as Google Zanzibar and SpiceDB treat authorization as a data problem. They solve the assembly dilemma by acting as a single, centralized graph database. However, they extract a heavy operational tax: all security vectors, organizational hierarchies, and team memberships across the enterprise must be continuously streamed and normalized into a single database format.

FDAE resolves this dilemma by decoupling the authorization specification, the **"What"**, from environment-specific execution, the **"How"**. It adopts the clean, graph-based syntax of Zanzibar for its configuration layout, allowing developers to define permissions as chains of relationships.

Crucially, FDAE removes the data-streaming tax by extending the specification with declarative data-source bindings. The engine acts as an intelligent distributed query planner. If contiguous relationship steps reside in the same SQL database, the engine automatically collapses them into a single, deeply nested query. If a relationship step crosses an asset boundary, the engine pauses, drops out of database execution, triggers a local Wasm host function or file lookup, and resumes the graph traversal inline.

## 2. Background: The Core Mental Model

The design of FDAE is built upon four fundamental principles:

- **The Authorization Formula:** A principal, such as a user, service, or group, has access to a resource, such as a document, widget, or API, if a valid logical chain of paths connects them together.
- **Structural Graph Hierarchy:** Principals and resources naturally form deep trees, such as sub-departments within parent departments, or reporting loops inside a management chain.
- **Data Fragmentation Reality:** In real-world microservice environments, the data representing these chains is scattered. Core metadata might live in a local SQLite file, reporting chains might live in an external HR system, and compliance constraints might reside in local configurations.
- **Unified Routing Engine:** FDAE treats your existing storage layout as a virtual graph database. If all rows happen to be migrated into a single Zanzibar-style tuple table, the engine executes flat graph traversals. If the data remains fragmented, the engine abstracts the physical layer and unifies execution under a single configuration profile.

## 3. Comprehensive Schema Specification (No-Parser AST Config)

To eliminate the runtime overhead and engineering debt of custom string lexers and text parsers inside the Wasm host framework, the DSL is written as a fully structured configuration tree. Deserializing this file directly produces the actionable Abstract Syntax Tree (AST) for the query planner.

```yaml
version: "fdae/v2alpha1"

# ------------------------------------------------------------------------------
# REGISTRY: Mapping Logical Keys to Physical Storage Engines
# ------------------------------------------------------------------------------
data_sources:
  sqlite_db:
    driver: "sqlite"
    connection: "file:app_state.db?mode=ro"
  hr_api:
    driver: "wasm_host_extension"
    export: "sys_fetch_management_chain"
  env_metadata:
    driver: "file_system"
    path: "/etc/platform/security_context.json"

# ------------------------------------------------------------------------------
# HIERARCHIES: Unbounded Graph Pathways
# ------------------------------------------------------------------------------
hierarchies:
  management_chain:
    data_source: "sqlite_db"
    table: "users"
    from_key: "id"
    to_key: "manager_id"
  department_tree:
    data_source: "sqlite_db"
    table: "departments"
    from_key: "id"
    to_key: "parent_id"

# ------------------------------------------------------------------------------
# DEFINITIONS: Objects, Data Joins, and Security Paths
# ------------------------------------------------------------------------------
definitions:
  user:
    data_source: "sqlite_db"
    table: "users"
    pkey: "id"

  department:
    data_source: "sqlite_db"
    table: "departments"
    pkey: "id"
    relations:
      # Declarative data map joins replace raw SQL strings
      member:
        target_object: "user"
        join_table: "dept_members"
        from_key: "dept_id"
        to_key: "user_id"
      manager:
        target_object: "user"
        data_source: "hr_api"

  document:
    data_source: "sqlite_db"
    table: "documents"
    pkey: "id"
    relations:
      creator:
        target_object: "user"
        join_column: "creator_uuid"
      parent_dept:
        target_object: "department"
        join_column: "owner_dept_id"

    # Permission logic structures representing boolean graph steps
    permissions:
      view:
        operator: "union"
        paths:
          # Path Option 1: Direct ownership check
          - - type: "relation"
              name: "creator"
            - type: "terminal"
              value: "caller"
          # Path Option 2: Transitive manager chain check
          - - type: "relation"
              name: "creator"
            - type: "hierarchy"
              name: "management_chain"
            - type: "terminal"
              value: "caller"
          # Path Option 3: Department governance check
          - - type: "relation"
              name: "parent_dept"
            - type: "relation"
              name: "manager"
            - type: "terminal"
              value: "caller"
```

## 4. Operational Scenario & Context Setup

To illustrate the engine's behavior, consider the following real-world scenario evaluated against the configuration file above.

### The Clear English Security Rule

"Alice wants to view a document. She is authorized if she is the direct creator, OR if she is an upward manager in the reporting line above the creator, OR if she is explicitly registered as the manager of the department that owns the document."

### Active State of the Application Context

- **Target Resource:** `document_abc123`
- **Logged-in Identity (Caller):** `user:alice`
- **Database Record (`documents` table):** `id = "document_abc123"`, `creator_uuid = "user:bob"`, `owner_dept_id = "department:engineering"`
- **Database Record (`users` table hierarchy):** `user:bob` reports to `user:charlie`, who reports to `user:alice`
- **External HR System State:** The designated manager for `department:engineering` is resolved by hitting an external Wasm module hook, which currently resolves to `user:david`

## 5. Engine Evaluation Steps & Execution Breakdown

When the guest Wasm microservice calls the platform host function requesting access to the asset, the FDAE state machine executes the following lookahead, translation, and filtering phases:

```text
                  [WASM Guest requests document_abc123]
                                    |
                                    v
                 [Host Engine Ingests Definition Target]
                                    |
               +--------------------+--------------------+
               v                                         v
     (Evaluate Path Option 2)                  (Evaluate Path Option 3)
 [creator -> management_chain]            [parent_dept -> manager]
               |                                         |
     (Contiguous SQL Source)                  (Heterogeneous Sources)
               |                                         |
  Compiles to single optimized                  1. SQL Query resolves
  WITH RECURSIVE SQLite Query                      'department:engineering'
               |                                 2. Drops out to WASM Host
   Executes: Bob -> Charlie -> Alice               Extension to trace 'manager'
               |                                         |
        [MATCH DETECTED]                          [SKIPPED VIA SHORT-
    Engine short-circuits to ALLOW                     CIRCUIT LOGIC]
```

### Step 1: Lookahead Optimization Phase

The engine reviews the permission block for the `view` operation. It looks at Path Option 2:

```text
[creator -> management_chain -> caller]
```

It looks up the `creator` relation and notes that its `data_source` is `sqlite_db`.

It looks up the `management_chain` hierarchy and notes that its `data_source` is also `sqlite_db`.

Because these nodes are adjacent and share the same physical storage driver, the engine realizes it does not need to make multiple procedural round-trips. It flags this path segment for **Join Tree Collapse**.

### Step 2: Query Generation & Execution (The SQL Pushdown)

Instead of executing sequential queries, the engine merges the relationship hops into a singular, highly efficient, cycle-protected recursive SQLite query string:

```sql
WITH RECURSIVE graph_walk_cte AS (
    -- Base Case: Start with the creator_uuid extracted from document_abc123
    SELECT id, manager_id, ',' || id || ',' AS visited_track
    FROM users
    WHERE id = (SELECT creator_uuid FROM documents WHERE id = 'document_abc123')

    UNION ALL

    -- Recursive Step: Jointly climb the reporting tree
    SELECT child.id, child.manager_id, parent.visited_track || child.id || ','
    FROM users child
    INNER JOIN graph_walk_cte parent ON child.id = parent.manager_id
    -- Loop Guard: Enforce graph safety to prevent infinite CPU exhaustion
    WHERE parent.visited_track NOT LIKE '%,' || child.id || ',%'
)
SELECT 1 FROM graph_walk_cte WHERE manager_id = 'user:alice';
```

SQLite processes this transaction instantly in-process. The recursive loop traverses from Bob, finds Charlie, reaches Alice, and evaluates to a true state.

### Step 3: Global Logic Short-Circuiting

Because the root permission node joins the paths using a `union` (`OR`) operator, the engine detects that Path Option 2 has fully satisfied the security profile.

The engine instantly returns an **Allowed** state to the host platform. It completely bypasses checking Path Option 3, preventing unnecessary execution of the external HR Wasm module hook and saving valuable system resources.

### Step 4: Hybrid Boundary Dropout Handling (Fallback Path Execution)

If the recursive management query in Step 2 had failed to find Alice, the engine would have instantly transitioned its state runner to evaluate Path Option 3:

```text
[parent_dept -> manager -> caller]
```

The fallback path proceeds as follows:

- **SQL Execution:** The engine scans `parent_dept` inside `sqlite_db`, resolving the reference value column to `department:engineering`.
- **Boundary Crossing:** The engine reads the next path step, `manager`. It checks the config definition and flags that the target data source is `hr_api` (`wasm_host_extension`).
- **Context Freezing:** The engine halts SQLite operations, packages the intermediate value `department:engineering`, and marshals the text string across the Wasm runtime memory boundary.
- **Procedural Hook Execution:** It fires the native host extension function `sys_fetch_management_chain`, which queries the external system memory space and returns `user:david`.
- **Terminal Matching:** The engine resumes control, tests the returned string against the terminal caller value (`"user:david" == "user:alice"`), and issues a final **Denied** state.

## 6. Dual-Mode Capability: Point Checks vs. Data Filtering

FDAE natively handles both access verification and structural dataset filtering depending on how the host platform triggers the execution interface.

### Mode A: Point-In-Time Evaluation (Check)

When verifying a specific resource handle, such as "Can Alice view document 12?", the engine appends an absolute constraint to the generated SQL queries:

```sql
WHERE documents.id = :target_id
```

It outputs a swift binary **Allowed** or **Denied** flag back to the gatekeeper system proxy.

### Mode B: Relational Data Filtering (Lookup/Filter)

When an authorized user requests a dashboard index, such as "Show me a list of all documents I am allowed to see", the engine transforms its output behavior. Instead of executing isolated checks, it maps the compiled `WHERE EXISTS` security block as a global subquery wrapping the user's base table command:

```sql
-- Generated automatically by the engine to filter incoming dataset views
SELECT * FROM documents
WHERE EXISTS (
    WITH RECURSIVE graph_walk_cte AS (
        SELECT id, manager_id FROM users WHERE id = documents.creator_uuid
        UNION ALL
        SELECT u.id, u.manager_id FROM users u
        INNER JOIN graph_walk_cte parent ON u.id = parent.manager_id
    )
    SELECT 1 FROM graph_walk_cte WHERE manager_id = 'user:alice'
)
LIMIT 50;
```

This forces SQLite to perform index-level data filtering inside the file transaction layer, ensuring that the guest Wasm microservice receives an already pruned, authorized dataset payload with zero post-processing serialization penalties.

## 7. Platform Security & Performance Safeguards

To keep the engine operating with deterministic predictability inside resource-constrained runtime nodes, the host platform enforces three mandatory constraints:

- **Strict Parameter Isolation:** The engine's query compiler is strictly barred from utilizing runtime string formatting or raw token concatenation when assembling queries. All boundary context keys, caller IDs, and resource tokens MUST be passed using native parameterized binding positions, such as `?` or `:name`.
- **Deterministic Cycle Protections:** Every recursive configuration block generated by the engine code generator must include a path concatenation tracker (`visited_track`) to actively kill execution branches if cyclic loops are introduced in the user tables.
- **Instruction & Latency Watchdogs:** All compiled query transactions must execute alongside an active instruction cycle watchdog via SQLite's progress handler API (`sqlite3_progress_handler`). If a generated recursive join tree chokes the query planner or takes longer than 15 milliseconds of execution time, the transaction is immediately rolled back and aborted with a secure **Default Denied** state.
