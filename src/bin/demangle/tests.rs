use super::*;
use std::io::Cursor;

fn invoke(args: &[&str], input: &str) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut input = Cursor::new(input.as_bytes());
    let mut out = Vec::new();
    run(&args, &mut input, &mut out, &Demangler::new()).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn passes_plain_names_through() {
    assert_eq!(invoke(&["main"], ""), "main\n");
    assert_eq!(invoke(&["not_a_symbol"], ""), "not_a_symbol\n");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn demangles_itanium_arguments() {
    assert_eq!(invoke(&["_ZN3Foo3barEv"], ""), "Foo::bar()\n");
    // `_Z` is claimed as Itanium and then rejected, exercising the CLI
    // fallback rather than the C pass-through.
    assert_eq!(invoke(&["_Z"], ""), "_Z\n");
}

#[test]
fn reads_stdin_when_no_arguments_or_dash() {
    assert_eq!(invoke(&[], "main\nplain\n"), "main\nplain\n");
    assert_eq!(invoke(&["-"], "main\n"), "main\n");
}

#[test]
fn argument_mode_does_not_read_stdin() {
    assert_eq!(invoke(&["main"], "ignored\n"), "main\n");
    assert_eq!(invoke(&["-", "main"], "ignored\n"), "-\nmain\n");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn stdin_demangles_itanium_names() {
    assert_eq!(invoke(&[], "_ZN3Foo3barEv\n"), "Foo::bar()\n");
    assert_eq!(invoke(&["-"], "_ZN3Foo3barEv\n"), "Foo::bar()\n");
}
