! Fortran probe module: module data, procedures, derived type with a
! type-bound procedure and a finalizer, a generic interface, an entry and a
! `bind(c)` procedure. Compiled by `refs/fortran/extract-fortran-symbols.sh`.
module fprobe
  implicit none
  integer :: counter = 0
  integer, parameter :: answer = 42
  real, save :: scale_factor = 1.0

  type :: point
    real :: x, y
  contains
    procedure :: norm => point_norm
    final :: point_final
  end type point

  ! A type whose name contains an underscore and one written in mixed case:
  ! gfortran records both as `<module>_<Type>` with the first letter
  ! upper-cased and the rest lower-cased in the generated helpers.
  type :: my_type
    real :: value
  end type my_type

  type :: CamelCase
    real :: mixed
  end type CamelCase

  interface total
    module procedure total_one, total_two
  end interface total

contains

  subroutine hello()
    counter = counter + 1
  end subroutine hello

  integer function add(a, b) result(r)
    integer, intent(in) :: a, b
    r = a + b
  end function add

  real function point_norm(self)
    class(point), intent(in) :: self
    point_norm = self%x + self%y
  end function point_norm

  subroutine point_final(self)
    type(point), intent(inout) :: self
    self%x = 0.0
    self%y = 0.0
  end subroutine point_final

  integer function total_one(a) result(r)
    integer, intent(in) :: a
    r = a
  end function total_one

  integer function total_two(a, b) result(r)
    integer, intent(in) :: a, b
    r = a + b
  end function total_two

  pure elemental real function twice(x) result(y)
    real, intent(in) :: x
    y = 2.0 * x
  end function twice

  subroutine outer(n)
    integer, intent(in) :: n
    call inner(n)
  contains
    subroutine inner(k)
      integer, intent(in) :: k
      counter = counter + k
    end subroutine inner
  end subroutine outer

  subroutine with_entry(a)
    integer, intent(in) :: a
    integer :: b
    counter = counter + a
    return
  entry with_entry2(b)
    counter = counter + b
  end subroutine with_entry

  subroutine trailing_()
    counter = counter + 2
  end subroutine trailing_

  subroutine c_proc(x) bind(c, name="c_proc_binding")
    integer, intent(in) :: x
    counter = counter + x
  end subroutine c_proc

end module fprobe
