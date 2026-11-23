unit delphi_probe_delphi;

{ FPC `{$mode delphi}` fixture: the Delphi compatibility mode keeps the FPC
  linker-name scheme, so the backend does not need a second grammar.
  Compiled with: fpc -O- -gw delphi_probe_delphi.pas }

{$mode delphi}

interface

type
  TDelphiMode = class
    procedure Method(a: LongInt);
  end;

procedure DelphiModeProc(const s: AnsiString);

implementation

procedure TDelphiMode.Method(a: LongInt);
begin
end;

procedure DelphiModeProc(const s: AnsiString);
begin
end;

end.
