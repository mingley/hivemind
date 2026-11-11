# ADR-2: Direct module calls

Status: Accepted (supersedes ADR-1)

Synchronous domain flows between `orders` and `inventory` use direct
function calls. The in-process event bus is retired for these paths.
