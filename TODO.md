# Things to add

- [x] Add a flag that allows for the transformation of all assignment vectors so that the first item
      is assigned to district 1, so something like [2,2,4,4,3,1,1,3] would turn into [1,1,2,2,3,4,4,3].
      This will improve xben even further, but would technically alter the data

- [ ] Add ability to natively work with shapefiles and geojson files. Also have an eye towards
      working with SQL stuff from GerryDB

- [x] Make tests for all of the errors

- [x] Maybe change the encoder and decoder into things that are their own structs with
      implementations?

- [x] Make a special MCMC writer for ben that add a self-loop counter to the start of the next item.
      This will be really useful for reducing the size of any chain that has a high rejection ratio
      (e.g. reversible)

- [ ] Add an overwrite option to the `ben relabel`/`canonicalize`/`reencode` subcommands rather than
      just doing it by default

- [x] Change the `-s` flag in the "shapefile" parameter to a `-d` (now `--dualgraph`/`-d`, with
      `--shape-file` kept as a hidden alias)

- [ ] Add a reverse mode to `ben relabel` to make reverting the labeling a little bit easier for the
      end user

- [ ] Add a `jsonl` mode to `ben relabel` to relabel the `jsonl` file.

- [x] Add a method to `read` that allows for reading a chunk of assignment vectors. Will need a
      cursor to do this so that we can read ahead to the end and then chunk it up. Might want to make
      this into a struct that implements `Iterator`
  - Accomplished with a general SubSample thing that can be routed in easily.

- [x] Finish out the robust suite of tests for the MkvChain mode. The pipeline is already tested,
      but it probably would be good to duplicate all of the tests that were written for the standard
      mode even if the adaptation is really simple.
  - [x] A bunch more fuzzing tests added in for this

- [ ] Add a compression mechanism on CutEdges similar to NBEC from Todd Proebsting
