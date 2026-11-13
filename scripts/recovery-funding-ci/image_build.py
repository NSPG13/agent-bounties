"""Nonsecret, deliberately synthetic compiled image bindings for qualification only."""
import json
SERVICE='srv-aaaaaaaaaaaaaaaaaaaa'
STANDBY_REVISION='1111111111111111111111111111111111111111'
FUNDING_REVISION='2222222222222222222222222222222222222222'
def record(here,name,value):
    with (here/name).open('x') as output:
        json.dump(value,output,indent=2);output.write('\n')
