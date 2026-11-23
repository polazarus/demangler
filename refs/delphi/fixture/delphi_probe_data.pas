unit delphi_probe_data;

{ FPC compiler-data fixture: a unit with an initialization section, a
  finalization section, a unit variable and a thread variable, so the corpus
  covers the unit entry symbols and the unit-wide tables.
  Compiled with: fpc -O- -gw delphi_probe_data.pas }

{$mode objfpc}{$H+}

interface

var
  DataCounter: LongInt;

threadvar
  DataThread: LongInt;

implementation

initialization
  DataCounter := 1;
  DataThread := 2;

finalization
  DataCounter := 0;
end.
