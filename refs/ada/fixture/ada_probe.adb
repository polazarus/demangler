package body Ada_Probe is
   procedure Hello is
   begin
      null;
   end Hello;

   function Add (A, B : Integer) return Integer is
   begin
      return A + B;
   end Add;

   procedure Alpha is
   begin
      null;
   end Alpha;

   procedure Beta is
   begin
      null;
   end Beta;

   procedure Gamma (X : Integer) is
   begin
      null;
   end Gamma;

   function Total return Integer is
   begin
      return 0;
   end Total;

   procedure Over (X : Integer) is
   begin
      null;
   end Over;

   procedure Over (X : Float) is
   begin
      null;
   end Over;

   protected body Prot is
      procedure Set (X : Integer) is
      begin
         V := X;
      end Set;

      function Get return Integer is
      begin
         return V;
      end Get;

      entry Wait when Open is
      begin
         null;
      end Wait;
   end Prot;

   package body Gen is
      procedure Do_It (X : T) is
      begin
         null;
      end Do_It;
   end Gen;

   procedure Swap (A, B : in out T) is
      Tmp : constant T := A;
   begin
      A := B;
      B := Tmp;
   end Swap;

   task body Worker is
   begin
      accept Start;
   end Worker;

   procedure Nested is
      procedure Inner is
      begin
         null;
      end Inner;
   begin
      Inner;
   end Nested;
begin
   null;
end Ada_Probe;
