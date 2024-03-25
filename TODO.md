# Things to add

- [ ] Add auto-sort functionality at top to get the first vector to be nice.
    - Should probably just shift smallest to the front and keep things in-order
      as much possible so [1_0, 1_1, 2_0, 2_1, 2_2, 3_0, 1_2] should transform
      into [1_0, 1_1, 1_2, 2_0, 2_1, 2_2, 3_0]

- [ ] Add a flag that allows for the transformation of all assignment vectors 
  so that the first item is assigned to district 1, so something like
  [2,2,4,4,3,1,1,3] would turn into [1,1,2,2,3,4,4,3]. This will improve
  xben even further, but would technically alter the data

- [ ] Add ability to natively work with shapefiles and geojson files. Also have
  an eye towards working with SQL stuff from GerryDB

- [ ] Make tests for all of the errors

- [ ] Maybe change the encoder and decoder into things that are their own structs with
  implementations?
