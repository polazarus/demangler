! Generic module: a generic interface over two module procedures plus a module
! variable that holds a procedure pointer.
module fprobe_generic
  implicit none
  abstract interface
    real function scalar_op(x) result(y)
      real, intent(in) :: x
    end function scalar_op
  end interface
  procedure(scalar_op), pointer :: op => null()

  interface apply
    module procedure apply_real, apply_int
  end interface apply

contains
  real function apply_real(x) result(y)
    real, intent(in) :: x
    y = x
  end function apply_real
  integer function apply_int(x) result(y)
    integer, intent(in) :: x
    y = x
  end function apply_int
  subroutine set_op(f)
    procedure(scalar_op) :: f
    op => f
  end subroutine set_op
end module fprobe_generic
