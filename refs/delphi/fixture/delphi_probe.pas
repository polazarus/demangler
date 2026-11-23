unit delphi_probe;

{ FPC mangling fixture for refs/delphi.md.
  Compiled with: fpc -O- -gw delphi_probe.pas
  Every routine below is exported and contributes one linker symbol. }

{$mode objfpc}{$H+}{$modeswitch advancedrecords}

interface

type
  TProbeEnum = (peA, peB);
  TProbeSet = set of TProbeEnum;
  TProbeProc = procedure(a: LongInt) of object;

  TProbeRec = record
    X, Y: LongInt;
    procedure MoveBy(dx, dy: LongInt);
  end;

  PProbeRec = ^TProbeRec;

  TProbeClass = class
  public
    class constructor CreateClass;
    class destructor DestroyClass;
    constructor Create;
    destructor Destroy; override;
    procedure Plain;
    function Add(a, b: LongInt): LongInt;
    procedure Greet(const s: AnsiString);
    class function Build: TProbeClass; static;
    procedure OpenArray(const a: array of LongInt);
    procedure OpenArrayConst(const a: array of const);
    procedure WithRecord(const r: TProbeRec; p: PProbeRec);
    procedure WithEnum(e: TProbeEnum; const s: TProbeSet);
    procedure WithMethodVar(m: TProbeProc);
    procedure ManyParams(a: LongInt; b: Int64; c: AnsiString; d: Pointer;
      e: Double; f: Boolean; g: Char);
    procedure WithLocal;
  type
    TNested = class
      procedure Deep;
    end;
  end;

  TProbeChild = class(TProbeClass)
  public
    function Add(a, b: LongInt): LongInt; overload;
    function Add(a: AnsiString): LongInt; overload;
  end;

  generic TProbeBox<T> = class
    procedure Put(const v: T);
  end;

  TIntBox = specialize TProbeBox<LongInt>;

procedure UnitProc(a: LongInt);
function UnitFunc(const s: AnsiString): AnsiString;
procedure NoArgs;
procedure UnitOpenArray(const a: array of AnsiString);
procedure NestedOwner;
procedure Overloaded(a: LongInt);
procedure Overloaded(a: AnsiString);
procedure UsesGeneric(box: TIntBox);
procedure ManyStringParams(a: AnsiString; b: AnsiString; c: AnsiString;
  d: AnsiString; e: AnsiString; f: AnsiString);
procedure ProcedureWithALongNameThatForcesThePrefixCrcSubstitutionBecauseTheEnclosingPrefixExceedsTheHundredCharacterLimit;
operator + (const a, b: TProbeRec) r: TProbeRec;

var
  UnitCounter: LongInt;

implementation

class constructor TProbeClass.CreateClass;
begin
end;

class destructor TProbeClass.DestroyClass;
begin
end;

constructor TProbeClass.Create;
begin
end;

destructor TProbeClass.Destroy;
begin
  inherited;
end;

procedure TProbeClass.Plain;
begin
end;

function TProbeClass.Add(a, b: LongInt): LongInt;
begin
  Result := a + b;
end;

procedure TProbeClass.Greet(const s: AnsiString);
begin
end;

class function TProbeClass.Build: TProbeClass;
begin
  Result := nil;
end;

procedure TProbeClass.OpenArray(const a: array of LongInt);
begin
end;

procedure TProbeClass.OpenArrayConst(const a: array of const);
begin
end;

procedure TProbeClass.WithRecord(const r: TProbeRec; p: PProbeRec);
begin
end;

procedure TProbeClass.WithEnum(e: TProbeEnum; const s: TProbeSet);
begin
end;

procedure TProbeClass.WithMethodVar(m: TProbeProc);
begin
end;

procedure TProbeClass.ManyParams(a: LongInt; b: Int64; c: AnsiString;
  d: Pointer; e: Double; f: Boolean; g: Char);
begin
end;

procedure TProbeClass.WithLocal;
  procedure LocalOfMethod(q: LongInt);
  begin
  end;
begin
  LocalOfMethod(1);
end;

procedure TProbeClass.TNested.Deep;
begin
end;

function TProbeChild.Add(a, b: LongInt): LongInt;
begin
  Result := a + b;
end;

function TProbeChild.Add(a: AnsiString): LongInt;
begin
  Result := Length(a);
end;

procedure TProbeRec.MoveBy(dx, dy: LongInt);
begin
  X := X + dx;
  Y := Y + dy;
end;

procedure TProbeBox.Put(const v: T);
begin
end;

procedure UnitProc(a: LongInt);
begin
  UnitCounter := a;
end;

function UnitFunc(const s: AnsiString): AnsiString;
begin
  Result := s;
end;

procedure NoArgs;
begin
end;

procedure UnitOpenArray(const a: array of AnsiString);
begin
end;

procedure NestedOwner;
  procedure LocalProc(x: LongInt);
  begin
  end;
begin
  LocalProc(1);
end;

procedure Overloaded(a: LongInt);
  procedure LocalInOverload;
  begin
  end;
begin
  LocalInOverload;
end;

procedure Overloaded(a: AnsiString);
  procedure LocalInOverload;
  begin
  end;
begin
  LocalInOverload;
end;

procedure UsesGeneric(box: TIntBox);
begin
end;

procedure ManyStringParams(a: AnsiString; b: AnsiString; c: AnsiString;
  d: AnsiString; e: AnsiString; f: AnsiString);
begin
end;

procedure ProcedureWithALongNameThatForcesThePrefixCrcSubstitutionBecauseTheEnclosingPrefixExceedsTheHundredCharacterLimit;
  procedure LocalOfLongProcedure;
  begin
  end;
begin
  LocalOfLongProcedure;
end;

operator + (const a, b: TProbeRec) r: TProbeRec;
begin
  r.X := a.X + b.X;
  r.Y := a.Y + b.Y;
end;

initialization
  UnitCounter := 0;

finalization
end.
