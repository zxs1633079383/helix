# Data Model: 26 Gate 总控

## GateRecord

gateId、order、featureDirectory、prerequisiteGate、R1-R4、sourcePins、fixtureHashes、laneVerdicts、reviewVerdict。

## ProgramState

`Planned -> GateActive -> GateGreen -> NextGateActive -> 26Green -> AwaitingDeletionSelection -> FullRegression -> Closed`

任一 required fail/skip/hash drift 进入 Blocked；不得跳过前序 Gate。

## DeletionCandidate

path、kind、versionDomain、reverseReferences、risk、selectedByUser、deletionCommit、postDeleteEvidence。
