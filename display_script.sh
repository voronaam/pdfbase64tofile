
#!/bin/bash

cat page*.txt | base64 -d > /tmp/decoded.jpg
display /tmp/decoded.jpg &
