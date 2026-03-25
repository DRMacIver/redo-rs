redo-ifchange $1.in
(
	echo "#!/usr/bin/env python3"
	cat $1.in
) >$3
chmod a+x $3
