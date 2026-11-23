package Ada_Probe is
   procedure Hello;
   function Add (A, B : Integer) return Integer;
   procedure Alpha;
   procedure Beta;
   procedure Gamma (X : Integer);
   function Total return Integer;

   procedure Over (X : Integer);
   procedure Over (X : Float);

   protected type Prot is
      procedure Set (X : Integer);
      function Get return Integer;
      entry Wait;
      entry Fam (Idx : Integer);
   private
      V : Integer := 0;
      Open : Boolean := False;
   end Prot;

   generic
      type T is private;
   package Gen is
      procedure Do_It (X : T);
   end Gen;

   package Int_Gen is new Gen (Integer);

   generic
      type T is private;
   procedure Swap (A, B : in out T);

   procedure Int_Swap is new Swap (Integer);

   task type Worker is
      entry Start;
   end Worker;

   task Singleton;

   type Rec is record
      A : Integer;
   end record;

   type Enum is (One, Two);
end Ada_Probe;
