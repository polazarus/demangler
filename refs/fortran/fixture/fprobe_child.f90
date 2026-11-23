! Child module: uses the parent module and re-exports a procedure.
module fprobe_child
  use fprobe
  implicit none
contains
  subroutine do_child()
    call hello()
  end subroutine do_child
  integer function child_value(a) result(r)
    integer, intent(in) :: a
    r = total(a, a)
  end function child_value
end module fprobe_child
