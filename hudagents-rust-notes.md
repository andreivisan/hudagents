# Rust Notes (General Lessons)

This is a general summary of Rust concepts and patterns discussed in our chat. It is intentionally framework‑agnostic.

## Ownership, borrowing, and moves
- `&self` is a **shared borrow**. You can read fields but you cannot move them out, because that would leave the struct partially moved.
- If a type doesn’t implement `Copy`, using it by value **moves** it.
- To use a field without moving it, borrow it: `&self.field`.
- If you need to share ownership, use `Arc<T>` (thread‑safe) or `Rc<T>` (single‑threaded).
- If you need to mutate behind a shared reference, use interior mutability (`Mutex`, `RwLock`, `Cell`, `RefCell`).

## Tuple structs and `.0`
- A tuple struct like `struct NodeId(pub usize);` is accessed with `.0`.
- You can add a method for readability:
  ```rust
  impl NodeId {
      pub fn index(self) -> usize { self.0 }
  }
  ```

## Enums and pattern matching
- Enums model **sum types** (one of several variants).
- `match` is the idiomatic way to handle input variants:
  ```rust
  match input {
      Input::Audio(bytes) => { /* handle */ }
      _ => Err(MyError::InvalidInput),
  }
  ```

## `Result` and `?`
- `?` propagates errors and requires compatible error types.
- Use `From` conversions to make `?` work across error types.

## Error handling and chaining
- Implement `Display` for user‑friendly error messages.
- Implement `std::error::Error` to integrate with Rust error tooling.
- **Error chaining**: when your error wraps another error, return it in `source()` so callers can see the root cause.
  ```rust
  impl std::error::Error for MyError {
      fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
          match self {
              MyError::Io(e) => Some(e),
              _ => None,
          }
      }
  }
  ```

## `From` vs `Error`
- `impl From<OtherError> for MyError` allows `?` conversions.
- `impl std::error::Error for MyError` makes your error type compatible with the standard error trait.
- They are **complementary**, not replacements for each other.

## `Cow` (Clone‑On‑Write)
- `Cow<'a, str>` holds **either** a borrowed `&'a str` or an owned `String`.
- It avoids allocations when you only need to borrow, but can allocate if you need ownership.
- Use `as_ref()` to get a `&str` regardless of whether it’s borrowed or owned.
  ```rust
  use std::borrow::Cow;
  fn takes_cow(id: impl Into<Cow<'static, str>>) -> Cow<'static, str> {
      id.into()
  }
  ```

## `OnceLock`
- `OnceLock<T>` stores a value initialized **once** in a thread‑safe way.
- Good for expensive global initialization (e.g., checking external dependencies).
- On stable Rust, `get_or_try_init` is unstable, so store a `Result` inside `OnceLock`:
  ```rust
  use std::sync::OnceLock;
  static INIT: OnceLock<Result<(), String>> = OnceLock::new();

  fn init_once() -> Result<(), String> {
      let result = INIT.get_or_init(|| Ok(()));
      result.clone()
  }
  ```

## API ergonomics with `impl Into<T>`
- Taking `impl Into<T>` makes constructors and builders more flexible for callers.
- It reduces boilerplate at call sites and avoids unnecessary allocations.
- Use it for **inputs**, not necessarily for hot‑path functions where borrowing rules matter.
  ```rust
  fn new(name: impl Into<String>) -> Self { /* ... */ }
  ```

## Builder pattern (Rust style)
- Use a mutable builder to gather state, then `build()` returns an immutable result.
- Validation (like detecting cycles or invariants) belongs in `build()`.
- This gives a clean separation between construction and usage.

## Graph basics (Rust data structure patterns)
- Adjacency list representation is idiomatic and efficient: `Vec<Vec<NodeId>>`.
- Keep node data in a separate `Vec<Node>` and refer to them by `NodeId`.
- Store indegree counts in a `Vec<usize>` for Kahn’s algorithm.

## Kahn’s algorithm (layered execution)
- Start with all nodes of indegree 0.
- Process in waves: each wave becomes one “layer.”
- Decrement indegrees of neighbors; when indegree hits 0, enqueue.
- If processed nodes < total nodes, a cycle exists.

## Testing practices
- For tests that depend on external environment (paths, binaries), skip gracefully if missing.
- Prefer clear asserts over `unwrap()` when errors are expected.

## Performance and clarity tips
- Avoid re‑doing expensive initialization inside hot paths.
- Prefer explicit data formats in inputs to avoid ambiguity.
- Keep helpers as plain functions unless there’s a strong reason to elevate them to “nodes.”
