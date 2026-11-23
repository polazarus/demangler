(* Fixture for the OCaml corpus: value-name shapes and escape spellings. *)

let value_2 x = x + 1

let value__2 x = x + 2

let v' x = x + 3

let upper_Case x = x + 4

let _underscore x = x + 5

let () =
  ignore (value_2 1);
  ignore (value__2 1);
  ignore (v' 1);
  ignore (upper_Case 1);
  ignore (_underscore 1)
