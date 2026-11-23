(* Fixture for the OCaml corpus: unit-scope values, submodules and a functor.
   The corpus generator pins the symbols ocamlopt 5.3.0 emits for it. *)

let plain = 1

let unit_proc x = x + 1

let many a b c = a + b + c

let rec loop n = if n = 0 then 0 else loop (n - 1)

let closure_maker x = fun y -> x + y

let foo__bar x = x + 1

let upper_Case_Fun x = x * 2

let _leading__underscore = ref "s"

let counter = ref 0

let bump () = incr counter; !counter

module Inner = struct
  let inner_value = ref 2

  let inner_fun x = x * 2
end

module Nested = struct
  module Deep = struct
    let deep_fun x = x + 3
  end
end

module type S = sig
  val s_val : int
end

module F (X : S) = struct
  let from_functor = X.s_val
end

module Applied = F (struct
  let s_val = 4
end)

exception Boom

let raise_it () = raise Boom

let () =
  ignore (Sys.opaque_identity plain);
  ignore (unit_proc 1);
  ignore (many 1 2 3);
  ignore (loop 3);
  ignore (closure_maker 1 2);
  ignore (foo__bar 1);
  ignore (upper_Case_Fun 3);
  ignore !_leading__underscore;
  ignore (bump ());
  ignore (Inner.inner_fun !Inner.inner_value);
  ignore (Nested.Deep.deep_fun 1);
  ignore Applied.from_functor;
  ignore (try raise_it () with Boom -> 0)
