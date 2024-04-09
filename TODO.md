# Things to add

- [ ] Add a flag that allows for the transformation of all assignment vectors 
  so that the first item is assigned to district 1, so something like
  [2,2,4,4,3,1,1,3] would turn into [1,1,2,2,3,4,4,3]. This will improve
  xben even further, but would technically alter the data

- [ ] Add ability to natively work with shapefiles and geojson files. Also have
  an eye towards working with SQL stuff from GerryDB

- [ ] Make tests for all of the errors

- [ ] Maybe change the encoder and decoder into things that are their own structs with
  implementations?

- [ ] Make a special MCMC writer for ben that add a self-loop counter to the start of 
  the next item. This will be really useful for reducing the size of any chain that
  has a high rejection ratio (e.g. reversible)

- [ ] Add an overwrite option to `reben` rather than just doing it by default

- [ ] Change the `-s` flag in the "shapefile" parameter in `reben` to a `-d`

- [ ] Add a reverse mode to reben to make reverting the labeling a little bit
  easier for the end user

- [ ] Add a `jsonl` mode to reben to relabel the `jsonl` file.