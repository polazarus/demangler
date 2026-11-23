(* Fixture for the OCaml corpus: the linked program. Its symbols cover the
   runtime helpers, the runtime pseudo-units and the C section suffixes. *)

let () =
  ignore (Ocaml_probe.unit_proc 1);
  ignore (Ocaml_probe.foo__bar 1);
  ignore (Ocaml_probe.closure_maker 1 2);
  ignore (Ocaml_ops.( +! ) 1 2);
  ignore (Ocaml_ops.( ==! ) 1 2);
  ignore (Ocaml_names.value__2 1);
  ignore (Ocaml_names.v' 1);
  ignore (Ocaml_prime'.get ());
  ignore (Ocaml_pack.Ocaml_pack_base.base_fun 1);
  ignore (Ocaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun 1);
  print_string (string_of_int (List.length [ 1; 2; 3 ]))
