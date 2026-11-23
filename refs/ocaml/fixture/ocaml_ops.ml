(* Fixture for the OCaml corpus: operator members and their `$XX` escapes. *)

let ( +! ) a b = a + b

let ( ==! ) a b = a = b

let ( @@ ) f x = f x

let ( <=> ) a b = compare a b

let ( -| ) a b = a + b

let ( ~- ) a = -a

let ( .%() ) a i = a.(i)

let ( $% ) a b = a - b

let () =
  ignore (1 +! 2);
  ignore (1 ==! 2);
  ignore (( @@ ) (fun x -> x) 2);
  ignore (1 <=> 2);
  ignore (1 -| 2);
  ignore (~- 1);
  ignore ([| 1 |].%(0));
  ignore (1 $% 2)
