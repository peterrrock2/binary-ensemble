# Binary Ensemble Compression (BEN)

This library is built as an analogue to [PCompress](https://github.com/mggg/pcompress) and is
designed to help improve storage of general ensembles of redistricting plans.

More specifically, this package is designed to take canonicalized JSONL files which store ensembles
using lines of the form

```
    {"assignment": <assignment_vector>, "sample": <sample_number_indexed_from_1>}
```

and compress them into pure binary files.

## Usage

You may install the binary-ensemble package from the cargo package manager using

```
cargo install binary-ensemble
```

[Here](./example/small_example.jsonl) is a link to a small example file that you can use to see what
the binary-ensemble package is capable of.

Then you can run the package in one of the following modes (assuming that `~/.cargo/bin` is in your
path):

- Encode

```
ben -m encode small_example.jsonl # Outputs small_example.jsonl.ben
```

- XEncode

```
ben -m x-encode small_example.jsonl # Outputs small_example.jsonl.xben
```

- Decode

```
ben -m decode small_example.jsonl.ben -o re_small_example.jsonl # Outputs re_small_example.jsonl
```

- XDecode

```
ben -m x-decode -p small_example.jsonl.xben # Prints decoding to the console
```

- Read

```
ben -m read -n 4 small_example.jsonl  # Outputs [1,1,1,2,2,2,3,2,3,1,4,4,4,3,3,4]
```

- XZCompress

```
ben -m xz-compress small_example.jsonl # Outputs small_example.jsonl.xz
```

- XZDecompress

```
ben -m xz-decompress small_example.jsonl.xz # Outputs small_example.jsonl
```

There is also a `reben` CLI tool that is available through this package, but there is more
information about that in the [Relabeling](#relabeling-for-smaller-files) section

## How it works

There isn't a lot of complexity to the algorithm that we employ here; the power of the compression
comes from the fact that we use some of the expected information regarding the assignment vectors to
shrink the size of the compressed vector substantially.

The BEN compression format is a bit-level compression algorithm (as compared to a byte-level
compression seen in most compression applications) which works in two stages:

1. Run length-encoding (RLE)
1. Bit compatification

The first step is pretty simple: given an assignment like

```
[1,1,1,2,2,2,2,3,1,3,3,3]
```

We can encode this vector using an ordered pair $(value,, length)$ to get the vector

```
[(1,3), (2,4), (3,1), (1,1), (3,3)]
```

While not very efficient in the above example, in the majority of districting plans, if we order the
nodes in the assignment vector according to something geographical, (e.g. GEOID), then the savings
can be substantial.

The BEN standard then takes this vector and compactifies it into the following series of bytes:

```
00000010_  <- the maximum number of bits needed to store the assignment values (2)
00000011_  <- the maximum number of bits needed to store the length values (3)
00000000
00000000
00000000
00000100_  <- the number of bytes needed to store the entire vector (4)
01011_101
00_11001_0
1001_1101
1_0000000 <- the bit-packed assignment vector
```

### Relabeling for Smaller Files

Since `ben` uses RLE under the hood, anything that can be done to improve the likelihood of long
runs of the same assignment in the output assignment vector will improve the compression
substantially. In general, this just means that we need to sort the nodes in the JSON file in such a
way that they have a high likelihood of being grouped together in the final assignment. Of course,
there is no BEST way to do this, but, generally, sorting the JSON file according to geographic
markers is a pretty good way to go.

Consider, for example [this](./example/CO_small.json) dual-graph file for the state of Colorado.
This is a block-level dual-graph file containing ~140k nodes in no particular order. If we then use
this dual-graph file to generate 100k example plans and store the result in an XBEN file, we end up
with something like [this](./example/100k_CO_chain.jsonl.xben).

While this file is substantially smaller than the original JSONL file (which clocks in at a whopping
27GB), it is still not as small as we might like. However, we can improve the size of these files
with a little bit of relabeling.

Before we can see the gains that relabeling buy us, we will need to extract the XBEN file back into
a BEN file so that we can work with it (WARNING: The BEN file that this will generate is ~7GB, but
we will fix that soon), so we will need to run the command

```
ben -m decode 100k_CO_chain.jsonl.xben
```

The first thing that we can do is change up the labeling of our plans so that districts are labeled
in the order that they appear in the assignment vector. For example, if we have the assignment
vectors

```
[2,2,3,3,1,1,4,4]
[2,2,3,3,4,4,1,1]
```

we, as humans can see that these are just the same assignments with some numbers switched around.
However, the XBEN compressor (which uses LZMA2 for compression) does not have the context that we
have, so, it is necessary for us to help it along a little bit.

Here, we can make use of the `reben` (short for relabeling-BEN) CLI tool to do this for us:

```
reben -m ben 100k_CO_chain.jsonl.ben
```

This generally produces an improvement on the XBEN compression without fundamentally altering
anything about the underlying data (beyond the relabeling), so it's generally recommended that to
run things through `reben` before compressing into an XBEN format. In our running example, we can
then compress this file back down to an XBEN format using

```
ben -m x-encode 100k_CO_chain_canonicalized_assignments.jsonl.ben
```

DON'T ACTUALLY DO THIS, IT WILL TAKE OVER AN HOUR!!!

**NOTE:** Decoding is fast, but encoding with high compression does take time (maybe an hour or 2
with how big this ensemble is, but that is only because this file is on census blocks. Files with
VTDs tend to only take 10 minutes or so.)

However, this is not the only thing that we can do to make the file smaller. An often more effective
strategy is to sort the file by using some geographical information before we run the chain. Since
nearby geographical regions tend to be placed into the same districts as each other, sorting the
nodes in the original JSON file according to something like GEOID tends to produce exceptionally
short run-length encodings (and thus, exceptionally small BEN files). However, it is not always the
case that we have the foresight to do this, so the question then becomes "can we sort the vectors
after we have run the simulation already?" to which the answer is "of course!"

This is where the other aspects of `reben`come into play. If we would like to produce a new mapping
for our dual-graph files so that they are sorted according to some key value then we may use the
command

```
reben -m ben -s <dual-graph-file-name> -k <key-name>  <ben-file-name>
```

In our example, the CO_small.json file has the GEOID20 key that we would like to sort on, so we call
the command

```
reben -m ben -s CO_small.json -k GEOID20 100k_CO_chain_canonicalized_assignments.jsonl.ben
```

This will produce the files

- 100k_CO_chain_canonicalized_assignments_sorted_by_GEOID20.jsonl.ben (~550Mb)
- CO_small_sorted_by_GEOID20_map.json (a map file containing the new data)
- CO_small_sorted_by_GEOID20.json (a dual-graph file with the nodes shifted around)

Notice, our BEN file has now shrunk from ~7Gb to around 0.5Gb, which is pretty good! Now, we can
further compress this file using the `x-encode` mode of the `ben` CLI

```
ben -m x-encode 100k_CO_chain_canonicalized_assignments_sorted_by_GEOID20.jsonl.ben
```

And this will produce the file
`100k_CO_chain_canonicalized_assignments_sorted_by_GEOID20.jsonl.xben` which will only be ~6Mb! That
is over a 1000x improvement over the original BEN file, and over a 4500x improvement on the JSONL
file!

### Assumptions

The BEN compressor does make some assumptions about the data that the user should be aware of:

- When using the `ben` CLI tool, it is assumed that the assignments in the assignment vector are
  stored in the same order as the nodes in some JSON dual-graph file. While this seems to be the
  standard, it is incumbent on the user to make sure that they know which dual-graph file / node
  labeling produced these assignment vectors.

- When the samples are encoded into BEN formatting, the decoded samples will always start at 1.

- The maximum value of any assignment value is assumed to be 65535 (the largest number that can be
  stored in 16 bits) and, likewise, the longest a single assignment run length is assumed to be
  65535\. In all practical applications, this should not cause any issues unless the user is
  specifically looking at ways to split Idaho into congressional districts at the census block level
  between the years 2010-2020 and they make the decision to sort the dual-graph file according to
  the congressional assignments (and if you are doing that, 1. why? and 2. maybe sort the dual-graph
  file in some other way that is more meaningful using `reben` first). None of the other states can
  cause any issues for any of the state-wide races.

- The computer that is applying the BEN and XBEN encoding and decoding algorithms is assumed to have
  sufficient memory to store an entire assignment vector. This should not be an issue since any
  computer that is trying to extract information from these files is presumably also doing analysis
  with the ensemble of plans, but it is worth mentioning.

## More on the XBEN format

The XBEN (short for eXtreme-BEN) format is the ideal format to use for large data storage and for
the transferring of ensembles from user-to-user. Compared to BEN, XBEN uses an implementation of
[LZMA2](https://en.wikipedia.org/wiki/Lempel%E2%80%93Ziv%E2%80%93Markov_chain_algorithm) to further
take advantage of the repetition often present across ensembles of redistricting plans to further
reduce the size of the file.

This, as one might think from the name, the Lempel-Ziv Markov Algorithm (named because it uses a
Markov Algorithm under the hood, not because it is good at compressing data arising from a Markov
process), is particularly good at improving the compression ration of ensembles generated by Markov
methods like GerryChain or Forest Recom. It also works well at generally improving the compression
of BEN files. The LZMA2 algorithm is based off of the
[LZ-77](https://en.wikipedia.org/wiki/LZ77_and_LZ78#LZ77) algorithm which replaces repeated
occurrences of data with a single copy of that data appearing earlier in the data stream. LZMA2 then
uses a Markov process to dynamically determine the best Variable-Length Encoding (VLE) that can be
assigned to the most frequently occurring sequential bytes of data that appear within a data stream,
and then encodes the data using these codes (for a simplified idea of what is going on here, see
[Huffman coding](https://en.wikipedia.org/wiki/Huffman_coding)).

Of course, in order for the LZMA2 to work, we need for the data to be encoded using bytes instead of
the bit-packing method employed by BEN, so, when converting from a BEN file to an XBEN file (the bit
packing tends to produce data that looks like a random series of bits which is generally
incompressable), we actually unpack each assignment vector into an intermediate format known as
BEN32, which is an RLE that uses 32 bits to encode each assignment vector (using a big-endian
formatted u16 for the value and a big-endian formatted u16 for the length and a null u32 as the
separator). The LZMA2 algorithm is then able to make use of the repetition of the bytes across the
BEN32 file to substantially improve the compression.

**NOTE:** The decompression from the BEN to the BEN32 format only processes one RLE assignment
vector at a time, so the memory requirement for the extra compression using LZMA2 is not that large.

## When to use ben vs xben

The BEN file format is designed to be a usable format for working with ensembles of plans. That is
to say, it comes with some auxiliary functionality that allows for the user to read off the
assignment vector for a particular sample number easily, and thus can be used to "replay" an
ensemble coming from a Markov chain if desired.

The XBEN file format is designed to be used for storage only. The `ben` CLI tool has been built with
an emphasis on fast decompression, so any file that is stored as an XBEN file can quickly (in under
5 minutes) be extracted into a usable BEN format. Of course, the trade-off for this is that the
compression itself is fairly slow, and can sometimes take several hours to finish if the data is not
relabeled to improve the efficiency. However, considering any method used to create an ensemble of
plans is likely to take several hours anyway, the additional compression time to get a small XBEN
file should be mostly inconsequential in the grand scheme of things.

## Limitations

Since the BEN format and CLI tool is designed to work with general ensembles of districting plans,
it does come with some limitations. First and foremost, while BEN excels at the storage of ensembles
of plans built on census blocks, the compression ratios tend to be smaller when considering
ensembles of plans for things like VTDs or Tracts. In practice, since the assignment vectors for
these plans do not tend to be very long (maybe 10-20k), this is not that big of an issue, but it is
worth keeping that in mind.

In the event that an exceptionally small file is needed for compressing a districting ensemble
arising from a Markov Chain Monte Carlo method (e.g. Recom or Forest Recom) on larger subunits like
tracts, the [PCompress](https://github.com/mggg/pcompress) format, which employs a byte-level delta
encoding, is a good alternative choice.
