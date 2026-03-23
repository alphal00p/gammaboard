# TODO

## Platform
- [ ] move materializer completely out of config
- [ ] implement `madnis` sampler-aggregator as a parametrization

## Dashboard Auth
- [ ] keep steering APIs explicit (`pause`, `assign`, `unassign`, `append task`, `create run`) instead of generic patch endpoints

## Backend Cleanup
- [ ] decide on this: either make new of the runners very small and use the store to fetch everything, or take it completely parsed args, but not something in between! right now the new functions are a mess!
