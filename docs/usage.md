# Usage

Library:

```rust
use demangle::{Demangler, Language, Scheme};

let demangler = Demangler::new();
let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();

assert_eq!(demangled.scheme(), Scheme::Cpp);
assert_eq!(demangled.language(), Language::Cxx);
assert_eq!(demangled.to_string(), "Foo::bar()"); // allocates only here, in the caller

// Swift is demangled in-tree:
let swift = demangler.demangle("$s4main5helloSSyYaKF").unwrap();
assert_eq!(swift.to_string(), "main.hello() async throws -> Swift.String");

// Go names are classified and normalised in-tree:
let go = demangler.demangle("runtime.(*mLockProfile).captureStack").unwrap();
assert_eq!(go.scheme(), Scheme::Go);
assert_eq!(go.to_string(), "runtime.(*mLockProfile).captureStack");
assert_eq!(
    demangler
        .demangle("example.com/x/pkg%2ev1.Make")
        .unwrap()
        .to_string(),
    "example.com/x/pkg.v1.Make",
);

// Objective-C metadata and method display forms are rendered in-tree:
let objc = demangler.demangle("_OBJC_CLASS_$_NSString").unwrap();
assert_eq!(objc.scheme(), Scheme::ObjC);
assert_eq!(objc.to_string(), "Objective-C class NSString");
assert_eq!(
    demangler.demangle("-[Foo bar:baz:]").unwrap().to_string(),
    "-[Foo bar:baz:]",
);
```

Raw symbol bytes:

```rust
use demangle::Demangler;

let demangler = Demangler::new();
let raw = demangler.demangle_bytes(b"_ZN3Foo3barEv")?;
assert_eq!(raw.to_string(), "Foo::bar()");
```

Object-file symbol names are byte strings, not guaranteed UTF-8. `demangle`
takes `&str`; `demangle_bytes` accepts `&[u8]` and feeds every backend through
the raw-byte parser. Invalid UTF-8 in identifier text is retained and printed
as `\xNN`; a name that no backend recognizes is returned as escaped plain text
with `Scheme::Unknown`.

Explicit scheme (detection bypassed):

```rust
use demangle::{Demangler, Scheme};

let demangler = Demangler::new();

// Text form, matching `demangle`.
let ada = demangler.demangle_with_scheme(Scheme::Ada, "pkg__subp")?;
assert_eq!(ada.to_string(), "pkg.subp");

// Byte form, matching `demangle_bytes`.
let raw = demangler.demangle_bytes_with_scheme(Scheme::C, b"plain\xff")?;
assert_eq!(raw.to_string(), "plain\\xff");
```

`no_std` caller:

```rust
use core::fmt::Write;
use demangle::Demangler;

let demangler = Demangler::new();
let mut out = MyFixedBuffer::new();
let _ = write!(out, "{}", demangler.demangle("_ZN3Foo3barEv")?);
```

CLI:

```sh
# single symbol
demangle "_ZN3Foo3barEv"

# from stdin
cat symbols.txt | demangle -
```
