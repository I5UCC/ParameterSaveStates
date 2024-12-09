import dataclasses
import json

@dataclasses.dataclass
class Parameter:
    name: str
    value: any