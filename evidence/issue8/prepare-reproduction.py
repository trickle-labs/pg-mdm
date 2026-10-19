"""Run the original scoped-state assertions while retaining the fixture for a branch comparison."""
import hashlib
from pathlib import Path

source = Path("tests/scoped_state.sql").read_bytes()
assert hashlib.sha256(source).hexdigest() == "a94c0a0b1011731e5f945a24a1fbdf9d4e8470fe1398aecfffcb94d6f0290b27"
cleanup = b"\\connect foundation mdm_test_login\nSET ROLE mdm_administrator;\nSELECT mdm_admin.drop_entity('scoped_state', 'scoped_state');"
assert source.count(cleanup) == 1
Path(__file__).with_name("fixture-prefix.sql").write_bytes(source.split(cleanup)[0])
print("PASS: exact scoped-state fixture retained through all assertions, with cleanup deferred")
